# `SRC-12` Credentials done properly — Implementation Plan

**Status:** ready to build, part A first. The product owner signed the
feature page's decisions on 2026-10-14 ("setuju semua"). Written 2026-10-15
by the planner (Claude Opus) for developer agents, under the role split in
`AGENTS.md`. Size L: three parts (decision 13), each with its own gate and
review before the next starts.

**Base and branch:** the Phase 1 branch `feat/phase-1-rest` (draft PR #144).
Built on the phase branch itself, in its worktree
`/home/hv/lakehouse-sec27`, now that `SRC-11` is done there; `SRC-13` is
built beside it on a sub-branch in another worktree. Migration numbers
`0080` onward (`SRC-13` takes `0075`–`0079`).
**Spec:** `docs/core/specs/src-12.md`. **Feature page:**
`docs/core/features/credentials-done-properly.md`; D1 to D13 below are its
decisions. **Design record:** ADR 0002 and its four addenda; this plan adds
addendum 5 (task A0).

---

## 1. What exists today (read on 2026-10-14; the developer re-verifies every anchor)

| # | Spec row | Code |
| --- | --- | --- |
| F1 | At rest | Three credential kinds (`CredentialSource`, `lakehouse-store/src/connectors.rs`): `env:` variables, operator `file:`s, and `managed` files the API writes (`lakehouse-api/src/connector_secret_store.rs`: `0600`, `O_EXCL`, fsync, rename; stage → publish → commit/rollback with a `.bak` link). Plain text on the `lakehouse_connector_secrets` volume, mounted read-write in the API and read-only in the orchestrator, which reads files itself (`dagster/dispar_orchestrate/secret_resolver.py`). Two allowlists that must stay identical (`state.rs` `CONNECTOR_ALLOWED_SECRET_REF_PATTERNS`, `secret_resolver.py`). `SecretResolver` is the extension seam (`lakehouse-core/src/secret.rs`). No crypto crate besides `sha2`/`hmac`/`jsonwebtoken`. No `${X:?}` in the base compose file. |
| F2 | Secret managers | None. `vault:` appears only in comments, old seeds and refusal tests. |
| F3 | OAuth | REST `oauth2_client_credentials` exists: the orchestrator fetches a token once per `build_source`, never refreshes; the API test refuses ("cannot test … OAuth2 client-credentials"). No authorization-code flow, no connector callback route. |
| F4 | Rotation | `PUT /api/connectors/{id}/credential` (probe-first, managed only) and `PUT …/secret`; `SEC-14`'s re-point rule. Running steps keep the value they read; queued steps re-read. A crash between publish and commit can leave the `.bak` copy. `env:` values are read at start-up. |

## 2. Decisions as built (the planner's notes in italics)

Part A — encryption at rest, the key change, rotation (D4, D5, D9, D10, D11, D12):

- **D4.** AES-256-GCM, one installation key `CONNECTOR_SECRETS_KEY` (32
  random bytes, base64), given to the API and the orchestrator, `${…:?}` in
  `docker-compose.yml` for both, empty in `.env.example` with a comment on
  how to generate it. *Crates: `aes-gcm` (RustCrypto) for the API, the
  `cryptography` package's `AESGCM` for the orchestrator — both mainstream;
  the dependency check of the PR must stay green.* Each stored value:
  version byte, 96-bit random nonce, ciphertext and tag; the associated data
  binds the file name, so a file copied to another connector's name does not
  decrypt. *Both sides decrypt; only the API encrypts.*
- **D4, first start.** A plain managed file found at start-up is encrypted
  in place (write encrypted copy, fsync, rename), then the plain one is
  gone. Operator `file:` and `env:` credentials are the operator's and are
  not touched (D10).
- **D5.** `lakehouse-api rotate-secrets-key` (a subcommand of the API
  binary, run by the operator with the old and the new key): re-encrypts
  every managed file; until it has run, the old key still decrypts
  (`CONNECTOR_SECRETS_KEY_PREVIOUS`, read-only).
- **D9.** After a replace, no copy of the old value: the `.bak` path is
  removed on commit as today, and *at start-up any leftover temporary or
  backup file is removed* (and logged by name, never by content).
- **D10.** The connector page shows each credential's kind; for `env:` it
  says a restart is needed after a change.
- **D11, D12.** As the feature page: audit without values; fixed error
  texts (key missing, key wrong, file unreadable).

Part B — the four secret managers (D1, D2, D3, D11, D12): a `ChainedSecretResolver`
with one manager resolver chosen by the operator's environment; new ref
schemes (`vault:`, `aws-sm:`, `gcp-sm:`, `azure-kv:`) admitted by both
allowlists; "Reference a secret" in the console; Vault and AWS proven by
gates on local stand-ins, Google and Azure by tests against their documented
APIs (*not run against the real services*, written so). Planned in detail
after part A's review.

Part C — OAuth (D6, D7, D8): the REST authorization-code flow with PKCE,
refresh, revocation, and the client-credentials refresh. Planned in detail
after part B's review.

## 3. Tasks, part A

1. **A0. ADR 0002 addendum 5 (planner).** The key, the format, who encrypts
   and decrypts, the start-up encryption, the key change.
2. **A1. Encryption in the API.** A small module (`secret_crypto.rs` or in
   `connector_secret_store.rs` if it fits): seal and open with the format
   above; the store writes sealed values; the resolver opens them; the
   start-up pass that encrypts plain managed files and removes leftovers.
   Unit tests: round trip; wrong key; a file renamed to another connector's
   name fails; a plain file is encrypted at start-up and the plain copy is
   gone; a leftover backup is removed.
3. **A2. Decryption in the orchestrator.** `secret_resolver.py` opens sealed
   managed files with the same format (*a shared test vector file checked by
   both test suites proves the two agree*). Unit tests, no network.
4. **A3. Compose and config.** `CONNECTOR_SECRETS_KEY` with `${…:?}` for the
   API and the orchestrator; `CONNECTOR_SECRETS_KEY_PREVIOUS` optional;
   `.env.example`; the gates' override files get a CI-only key following the
   `ci-…-not-real-…` pattern (*and the gitleaks allowlist, if it needs an
   entry, gets one with the reason*). Proven by CI's `docker compose up`.
5. **A4. The key change command (D5).** The subcommand; tests.
6. **A5. Console (D10).** The credential's kind on the connector page; the
   restart note for `env:`.
7. **A6. Gate.** In an existing or new job (no existing job past 20
   minutes): a typed credential is stored encrypted (the gate reads the file
   from its own volume and checks it is not the plain value and does not
   contain it), the connector tests and loads; the key change command runs
   and the connector still loads; starting without the key is refused
   (compose `config` with the variable unset fails and names it).
8. **A7. Docs (planner).** Feature page, `CHANGELOG.md`, `BACKLOG.md`,
   `docs/plans/SRC-12-RESULT.md` part A.

## 3b. Tasks, part B — the four secret managers (planned 2026-10-15, after part A)

### Decisions as built (D1, D2, D3, D11, D12; the planner's notes in italics)

- **One manager per installation**, chosen by `SECRET_MANAGER` = `vault` |
  `aws` | `gcp` | `azure` (unset: none). Its connection settings are the
  operator's, in the environment of the API and the orchestrator:
  - Vault: `VAULT_ADDR`; sign-in by `VAULT_TOKEN`, or AppRole
    (`VAULT_ROLE_ID`, `VAULT_SECRET_ID`); KV version 2.
  - AWS Secrets Manager: `AWS_REGION` and the standard credential chain
    (instance or task role first; access keys in the environment
    otherwise); `AWS_ENDPOINT_URL_SECRETS_MANAGER` only for the gate's
    stand-in.
  - Google Secret Manager: `GOOGLE_APPLICATION_CREDENTIALS` (a service
    account file) or the platform's metadata identity; the project is part
    of the reference.
  - Azure Key Vault: `AZURE_TENANT_ID` with `AZURE_CLIENT_ID` and
    `AZURE_CLIENT_SECRET`, or a managed identity.
- **A reference is a new credential kind**, written by the API only:
  `vault:<mount>/<path>#<key>`, `aws-sm:<secret id>[#<key>]`,
  `gcp-sm:projects/<project>/secrets/<name>/versions/<version|latest>`,
  `azure-kv:<vault name>/<secret name>[/<version>]`. Both allowlists admit
  only the configured manager's scheme, and only when `SECRET_MANAGER` is
  set; *the two allowlists stay identical, checked by a shared test list as
  part A's vector is.*
- **Read at each use, no cache** (D1): every test, every listing, every
  run start. A manager that cannot be reached fails that use with a fixed
  text; nothing falls back to an older copy.
- **Both sides read** (D1): the API in Rust, the orchestrator in Python.
  *Prefer one small HTTP client per manager over a full cloud SDK where the
  manager's REST interface and its sign-in are simple (Vault, Azure,
  Google with a service-account token exchange); AWS needs request signing:
  use the `aws-sigv4` crate in Rust and `botocore`'s signer in Python, or
  the official SDKs if they pass `cargo deny` and add little. Every new
  dependency is named in the commit with its licence and why.*
- **The console** (D2): when a manager is set, the credential form offers
  "Reference a secret" with the fields of that manager's reference; the API
  reads the reference once and tests the connection before saving, as a
  typed credential is tested today; only the reference is stored. Typing a
  value stays possible unless `CONNECTOR_TYPED_CREDENTIALS=false`.
- **Proof** (D3): gates against Vault's development server and an
  AWS-compatible stand-in (a `moto` server container), each a step in a job
  of its own; Google and Azure by tests against recorded responses of their
  documented REST interfaces, and the result document says they were not
  run against the real services.
- **Errors** (D12): fixed texts — the manager cannot be reached, the
  product may not sign in to it, the reference does not exist, the product
  may not read it, the value is not a single text. Never a manager's own
  text.
- **Part A's open review item, done first** (`SRC-12 review 1 SHOULD-FIX
  1`): a credential file that cannot be opened with the key now says so
  with its own fixed text ("the stored credential cannot be decrypted with
  this installation's key"), instead of the generic "resolved to nothing".

### Tasks

1. **B0.** Part A's SHOULD-FIX above.
2. **B1. The seam.** A manager resolver behind `SecretResolver` and its
   Python twin, chosen once at start-up from `SECRET_MANAGER`; the reference
   parsers; both allowlists and the shared test list; the fixed errors.
3. **B2–B5. One commit per manager**, Rust and Python together, each with
   unit tests against recorded or mocked HTTP (no network): Vault, AWS,
   Google, Azure.
4. **B6. Saving a reference.** The credential routes accept a reference
   for a configured manager (validated, read once, connection-tested,
   audited without the reference's secret parts); the console form; the
   connector page shows "Secret manager: <kind>".
5. **B7. Gates.** Vault and AWS stand-in steps: store a source password in
   the manager, reference it, test and load a connector, change the secret
   in the manager, the next run uses the new value, and the credentials
   volume holds no copy. Registered as part A's gate was.
6. **B8. Docs (planner).** ADR 0002 addendum 6, `docs/OPERATIONS.md`, the
   feature page, `CHANGELOG.md`, `docs/plans/SRC-12-RESULT.md` part B.

## 3c. Tasks, part C — OAuth for REST connectors (planned 2026-10-15, after part B's review)

### Decisions as built (D6, D7, D8, D11, D12; the planner's notes in italics)

- **The flow (D6).** OAuth 2.0 authorization code with PKCE (S256) for
  the REST connector. The connector's auth kind `oauth2_authorization_code`
  holds: the provider's sign-in address, token address, optional revocation
  address, scopes, and the app's client id; the client secret is a
  credential like any other (typed, encrypted by part A, or a part B
  reference). *Every address the product dials (token, revocation) must be
  `https` and passes the same address check as connection tests; the
  sign-in address is only handed to the browser.*
- **Sign in.** `POST /api/connectors/{id}/oauth/start` (`connector:manage`)
  returns the sign-in address with a `state` that is random, one-time,
  expires after 10 minutes, and is bound to the connector and to the user
  who started it; the PKCE verifier stays on the server with it (a table,
  migration `0080`+). The console opens it in a new window. The provider
  returns to the product's own callback,
  `GET /api/connectors/oauth/callback?code&state`, which requires the same
  signed-in user, consumes the state, exchanges the code, stores the
  refresh token as the connector's credential (sealed, part A) and closes
  the window with a fixed message. The callback address is shown on the
  form, to copy into the provider's app settings. Any mismatch (unknown or
  expired state, another user, provider error) ends with a fixed text and
  stores nothing.
- **Refresh (D7).** Loads ask the token address for an access token with
  the refresh token before the first request and again when the source
  answers 401 once; access tokens are never stored. If the provider issues
  a new refresh token, the orchestrator sends it to the API on a new
  service route (`ingest:read`, `require_orchestrator`, as the run reports)
  and the API seals it in place of the old one; *the orchestrator never
  writes the credentials volume.* A refused refresh fails the run with the
  fixed text "sign in again" (`sign_in_refused`, not retried, `SRC-11`), and
  the connector page shows the Sign in button. The client-credentials kind
  gets the same "fetch again on expiry or a 401" rule. The connection test
  performs the token request, so OAuth connectors can be tested.
- **Sign out (D8).** Removing the credential or deleting the connector calls
  the revocation address when one is set and says so; when none is set, the
  page says the provider cannot be told and the token stays valid there
  until it expires.
- **Errors (D12)** are fixed texts; a provider's own error text or
  description is never shown or stored.

### Tasks

1. **C1. Store and routes.** The pending-state table; `oauth/start`,
   `oauth/callback`, the refresh-token update route; the auth kind in the
   ingest spec with validation (https, address check); `POLICY_TABLE` and
   `tests/route_auth.rs` (the callback needs a signed-in user, not
   `connector:manage`, and checks it is the one who started). Route tests
   with a mocked provider, including every refusal.
2. **C2. Orchestrator.** `adapters/rest.py`: the refresh-token grant, the
   401 retry once, the rotation report; the same expiry rule for client
   credentials. Unit tests, no network.
3. **C3. Test and revoke.** The connection test's token request; revocation
   on removal and delete.
4. **C4. Console.** The auth kind's fields, the callback address to copy,
   the Sign in button and window, "sign in again" on the connector page.
5. **C5. Gate.** A provider the gate runs itself (as `SRC-17`'s receiver):
   an authorize endpoint that approves at once and redirects with a code,
   a token endpoint issuing 5-second access tokens and rotating refresh
   tokens, a revocation endpoint; a REST stub that requires the bearer
   token. Steps: the gate drives the sign-in as a browser would; a load;
   a second load after the access token expired (refresh used, the rotated
   refresh token stored, still sealed); revocation called on removal; a
   wrong `state` and a replayed `state` refused. Its own job, registered as
   before.
6. **C6. Docs (planner).** ADR 0002 addendum 7, `docs/OPERATIONS.md`, the
   feature page, `CHANGELOG.md`, `docs/plans/SRC-12-RESULT.md`.

## 4. Build limits on this machine

As the `SRC-11` and `SRC-13` plans: no `cargo test`/`build`/`nextest`/`clean`,
no containers, no compiling cargo command under 12 GB free; check `free -g`
before heavy test runs (about 11 GB RAM, shared). CI on #144 is the proof.

## 5. Out of scope

Writing secrets into a manager from the console; a Google Sheets loader;
CDC credentials; changing passwords at the source; container hardening
beyond what part A needs.

## Handoff

*(developer)*

### Part A (developer)

**Commits** (branch `feat/phase-1-rest`, not pushed): `f0fe3cd4` A1 encryption in the API; `068eb062` A2 orchestrator opens sealed files; `9f743fce` A3 compose, config, CI keys, gitleaks; `0ed44de8` A4 the key-change command reaches the binary; `e0a4e0e4` SRC-11 gate fix (review 3 SHOULD-FIX 1: step d waits for its Dagster run to be final); `6209593e` A6 gate (`g6-credentials`); `90d2c020` A5 console. This handoff is the commit after.

**Where things are.** The key-change command itself (`rotate-secrets-key` in `main.rs`, `ConnectorSecretStore::reseal_all`) landed with A1 because the start-up pass shares the file walk; A4 only lets it be run (`rust/entrypoint.api.sh` passes a command to the binary). Format, keys and errors: `lakehouse-api/src/secret_crypto.rs`. The managed refs are read in `state.rs` through the new `FileSecretResolver::read_confined` (lakehouse-core), so the confinement rule is not duplicated. `main` refuses to serve without a usable key, then runs the start-up pass (seals plain managed files, removes `.connector_managed_*.tmp|bak` leftovers, logs names only).

**Commands run and outcomes.**
- `cargo fmt --check` (workspace): clean.
- `cargo clippy -p lakehouse-store -p lakehouse-api --all-targets -- -D warnings` (includes lakehouse-core): clean, on the final Rust state. Earlier A1 run for api+core: clean after fixing two lints.
- `(cd dagster && pytest dispar_orchestrate -q -m "not loader")`: 927 passed (run after A2); `test_secret_resolver.py` 25 passed.
- `bun run typecheck`: clean. `bun test src/features/connectors`: 268 pass, 0 fail. `bun run lint`: 0 errors, 6 warnings, none in touched files.
- `bash scripts/ci/tests/test_required_gate.sh`: 46 passed, 0 failed.
- `ops/lint/check_*.py` all four: ok. YAML loads: `docker-compose.yml`, `ci.yml`, `deploy-staging.yml`, `ops/g6/docker-compose.src12.override.yml`; TOML load of `.gitleaks.toml`; `sh -n rust/entrypoint.api.sh`; `py_compile` of both gate files.

**Not run.** `cargo test` (any crate): the new Rust tests (secret_crypto, connector_secret_store, state resolver, store `credential_source_of_ref`, the two integration tests that now read sealed files) compile under clippy `--all-targets` but were never executed. No docker: `docker compose config`/`up` for the changed compose file and for `ops/g6/docker-compose.src12.override.yml`, and the `g6-credentials` gate, never ran. `cargo deny`, `gitleaks`, `cargo build --locked` not run (the lockfile only gained the `lakehouse-api` -> `aes-gcm` edge; `aes-gcm 0.10.3` was already locked via `iceberg`). `cryptography==50.0.2` was found in the local venv, not checked as installable in the image.

**Shared test vector.** `rust/crates/lakehouse-api/tests/fixtures/connector_secret_vector.json`, written once with Python `cryptography`'s `AESGCM` (key = bytes 0..31, nonce = bytes 100..111, associated data = file name), version byte 0x01 prepended. Opened by `secret_crypto.rs` tests (including "does not open under another file name") and by `test_secret_resolver.py`.

**CI keys.** CI `.env` key = base64 of the 32 ASCII characters `ci-connector-key-not-real-000000` , in all 14 "Write CI-only .env" steps and in the new job; the job's "new" key = base64 of `ci-connector-key-rotated-0000000`; the API integration tests' key = base64 of 32 bytes of 0x2a (`tests/common/mod.rs`, default for every test app unless overridden). SEC-36 superseded this: the literals are gone (CI generates keys with `openssl rand`, the vector describes its key as bytes, the test key is encoded at run time) and the historical findings are baselined by fingerprint in `.gitleaksignore`; the `.gitleaks.toml` allowlist entries were removed. `deploy-staging.yml` now requires repository secret `STAGING_CONNECTOR_SECRETS_KEY` by name.

**Tests added or changed.** New: `secret_crypto` (vector, wrong key, other name, previous key, flipped byte, not-a-seal, key checks), store (start-up pass seals and removes leftovers, unknown-key file left, `reseal_all`, no key means no write, copied file does not open), state (managed file opened, fail closed), Python (vector, previous key, malformed/missing key, plain file refused, end to end), store `credential_source_of_ref`, console detail/edit page text. Changed: store tests now read through the key; `connector_repoint.rs` and `connector_not_testable.rs` read the stored file sealed; `tests/common` gives every test app a key.

**Decisions of mine to check.** (1) Unsealed managed files fail closed in both resolvers, so between a first start with the key and the API's start-up pass an orchestrator read of an old plain file is refused. (2) A sealed file under an unknown key is reported (`unreadable`) and left alone; "looks sealed" = first byte 0x01 and at least 29 bytes. (3) Open failures surface to a user as the existing "resolved to nothing" plus a fixed log line (decision 12 text, ref name only); no new `SecretError` variant. (4) `reseal_all` compares inode, size and mtime just before rename and reports a changed file as failed; a small race remains if a credential is saved during the command, so run it while none is. (5) `docker compose run` of the API passes through `depends_on` of the stack. (6) A6 is a new job (`g6-credentials`, limit 25): `g6-ops` already has limit 30. The g6 override makes `/run/secrets` read-only in the API, so a second override gives the API the writable volume (compose merges `volumes` by mount path; this is the part most likely to need a CI iteration).

**Most likely CI failures.** (a) `g6-credentials`: the volume-merge override, the `run ... g6-ops-test-runner "<command>"` form (entrypoint `/bin/sh -c`), `pip install` of `requests`, the `credential.primary` field in the create response for a managed credential, timing of `up --force-recreate`. (b) Any job I missed that runs `docker compose` without the key now fails at config time with the new `:?` message (all 14 `.env` writers and the staging deploy were updated). (c) `cargo deny`/gitleaks on the new dependency edge and key literals. (d) Rust unit/integration tests never run locally. (e) `dagster` image build if `cryptography==50.0.2` does not resolve for python 3.12-slim.

**Planner items (A7 and docs).** `docs/OPERATIONS.md` has no key-change how-to yet (command in the A4 commit message); the feature page and `CHANGELOG.md`; `STAGING_CONNECTOR_SECRETS_KEY` must exist before the next staging deploy (others' job); the README/quick-start `cp .env.example .env` now needs a generated key.

### Part B (developer)

**Commits** (branch `feat/phase-1-rest`, not pushed, in plan order): `71444280` B0 fixed text for a stored credential that cannot be opened (`SecretError::Failed`, `SecretFailure`); `7e12a95f` B1 the seam, the reference grammar, both allowlists, the shared list `rust/crates/lakehouse-api/tests/fixtures/secret_manager_refs.json`; `6d9f29da` B2 Vault; `b97d7ebc` B3 AWS Secrets Manager; `8b3bf87c` B4 Google Secret Manager; `e5981f66` B5 Azure Key Vault; `04732d83` B6 saving a reference (API, store, config, compose, console); `5b995194` B7 gate `g6-secret-managers`. This handoff is the commit after. B8 is yours.

**Where things are.** API: `lakehouse-api/src/secret_manager/{mod,refs,vault,aws,gcp,azure}.rs`; `ManagerResolver` sits inside `state::ConnectorSecretResolver`; the allowlist is `state::connector_allowed_patterns(manager)` (the seven fixed patterns plus the configured manager's `<scheme>*`, none when `SECRET_MANAGER` is unset). Orchestrator: `dagster/dispar_orchestrate/secret_manager.py` (grammar, allowlist, fixed texts, `request_json`) and `secret_manager_{vault,aws,gcp,azure}.py`, called from `secret_resolver.resolve_secret_ref`. Credential routes: `routes/connectors.rs` (`CredentialSlotBody` is now `{kind?, value?, reference?}`; `slot_changes_with`, `resolve_references`, `credential_audit_details`, `check_create_credential`, `credential_settings`). Console: `src/lib/connectors/secret-reference.ts`, `secret-reference-field.tsx`, `CredentialFields`.

**Commands run and outcomes.**
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` (after `find crates -name '*.rs' | xargs touch`, shared target `/home/hv/.cache/lakehouse-src6-target`, jobs 2), on the final Rust state (`04732d83`; B7 touches no Rust): `Finished`, exit 0. Each Rust commit was also checked with `cargo clippy -p lakehouse-api` (B6 with `-p lakehouse-store`), `--all-targets`, on its own tree; B1 and B2 were first checked on a tree that already held the next step's files and then re-checked on the committed state.
- `cargo fmt --check`: clean.
- `(cd dagster && pytest dispar_orchestrate -q -m "not loader")`: 1093 passed, 258 deselected. New files: `test_secret_manager_refs.py` 42, `_vault.py` 13, `_aws.py` 19, `_gcp.py` 18, `_azure.py` 15 passed; `test_secret_resolver.py` 25 (assertions on the texts changed, see below).
- `bun run typecheck`: clean. `bun test src/features/connectors src/lib/connectors`: 342 pass, 0 fail. `bun run lint`: 0 errors, the same 6 warnings, none in touched files.
- `bash scripts/ci/tests/test_required_gate.sh`: 48 passed, 0 failed. `ops/lint/check_*.py` (4): ok. YAML loads: `docker-compose.yml`, `.github/workflows/ci.yml`, `ops/g6/docker-compose.src12-managers.override.yml`, `ops/g6/docker-compose.src12.override.yml`. `py_compile` of `ops/g6/src12_managers_test.py`.

**Not run.** `cargo test` (any crate): all new Rust tests (refs, mod, vault, aws, gcp, azure unit tests with `wiremock`; the config, store and `routes/connectors.rs` unit tests; the integration test `tests/connector_secret_reference.rs`, which needs the database container) compile under clippy `--all-targets` but never ran. No docker: `docker compose config`/`up` for the changed `docker-compose.yml` and for the new override, and the gate `g6-secret-managers`, never ran. `cargo deny` is not installed here, `gitleaks` not run. The image tags `hashicorp/vault:1.18.5` and `motoserver/moto:5.0.9` were typed from memory and not pulled: if one does not exist, the gate fails at `up`, and the tag is the fix. The Google and Azure clients were never run against the real services (said in each commit, in `docs/CI.md`, and to say again in `SRC-12-RESULT.md`).

**Dependencies added.** No new crate or package in the lockfile or in `pyproject.toml`. `lakehouse-api` gained two Cargo edges to crates already locked: `hmac 0.13.0` (MIT or Apache-2.0; the SigV4 signer) in `Cargo.toml` and `Cargo.lock`, and `jsonwebtoken` (MIT; Google's JWT) moved from dev-dependencies to dependencies (no lock change). No small-client choice needed an SDK, so nothing was refused or stopped. Python: the SigV4 and JWT signers use `hmac`/`hashlib` and `cryptography` (already pinned); no `boto3`, `google-*`, `azure-*`. The local venv has everything the tests need (`requests`, `cryptography`; `botocore` is there too but is used by no test: the botocore signature in the tests is a constant produced once). The gate installs `boto3` with `pip` in its runner step only (Apache-2.0). CI images: `motoserver/moto` Apache-2.0; `hashicorp/vault` is BUSL-1.1 from 1.15 on, used only as a CI fixture and not distributed; OpenBao would be the drop-in.

**What is treated as secret, and why.** Secret: the value a manager returns; the operator's sign-in (`VAULT_TOKEN`, `VAULT_SECRET_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN` and role credentials, the service-account private key and the signed assertion, `AZURE_CLIENT_SECRET`); every token those produce. They live in `SecretValue` or in `SecretManagerSettings`, whose `Debug` prints the manager only; they are not cached (each read signs in again and drops the token), logged, returned, or audited. Not secret: a reference (Vault mount/path/key, a secret name or ARN, a Google project/secret/version, an Azure vault/secret/version): it names a place. So a reference is audited as it is (who pointed which connector at which secret), and is logged by the resolver's warn line. Differs from `rotate_secret`'s earlier "never a ref name" convention, which was about derived names; the derived managed name stays out.

**Decisions of mine to check.**
1. *AWS credential order.* The plan says "instance or task role first". Built: a role the environment names (`AWS_CONTAINER_CREDENTIALS_*`) first, then keys in the environment, then the instance role through IMDSv2 (2 s timeout) last. The standard chain tries keys first, and probing IMDS before keys costs a timeout per read on a host that has none. Web identity (IRSA) and profile files are not built: a limit for the result document and `OPERATIONS.md`.
2. *Any `connector:manage` user can reference any secret the product's manager identity can read.* The grammar limits the shape, not the place. A user who also chooses the connector's host can send what a reference reads to that host in the connection test (the SSRF guard still applies). The mitigation is the operator's: give the product an identity whose policy covers only the secrets meant for connectors. `.env.example` says so; the plan has no per-installation prefix allowlist (for example `SECRET_MANAGER_ALLOWED_PREFIXES`), and I did not add one. Your call whether it is a follow-up or a blocker; it is the same class as `WS3 plan review X1`, moved from env names to manager paths.
3. *Fixed texts for failures reach users by two routes.* `SecretError::Failed` (new variant; its `Display` is the fixed text, the reference is a field and not in the message). In the console a refused reference is a 422 "the credential was NOT saved: <text>"; in a connection test the probe prefixes "could not resolve the connector's credential: <text>". The Python side raises `SecretRefRejected("secretRef '<ref>': <text>")`, so a failed run's log names the reference and the fixed text. Part A's key-missing text in Python no longer names `CONNECTOR_SECRETS_KEY` (it goes to the log instead): the Python tests changed accordingly, and the Rust `CryptoError` messages are unchanged.
4. *The re-point path takes references too.* `PUT .../ingest-spec` with `credential` shares `slot_changes_with`, so a reference can ride with a change of target. Resolution (the call to the manager) happens after the SEC-14 refusal and, on this route, after the address is approved.
5. *Creating a connector by reference is two calls in the console.* `POST /api/connectors` refuses the new `reference` source (400, explains); the create page creates the connector empty, saves the connection settings, sets the reference through the probe-first route, then tests. If the reference fails the connector exists without a credential and the page says so; the same reference can be set from its edit page. A one-request create would need `create_connector` to take explicit names: not in the plan.
6. *Compose passes the managers' variables as `${X:-}` to both services*, so empty strings reach the containers (botocore ignores an empty `AWS_ACCESS_KEY_ID`; both services ignore empty settings). A Google key file has to be mounted by the operator into both services at the same path; the compose file does not mount one.
7. *`kind` is optional in a slot that carries a reference*, ignored there. A slot that sends both a value and a reference, or neither, is a 400.
8. *The settings route.* A new `GET /api/connectors/credential-settings` (policy `connector:manage`) feeds the form; the other option, changing `GET /api/connectors/types`' shape, would have broken its other consumer.
9. Vault is read as KV v2 only; `VAULT_NAMESPACE` is not supported. Azure: only the public cloud's `vault.azure.net`. Google: only `service_account` key files and the metadata identity. All listed as limits.

**Tests added or changed.** Added: grammar and allowlist cases shared by Rust and Python through `secret_manager_refs.json` (9 valid, 24 refused references, 14 allowlist cases, base allow/refuse lists, the eight texts); per-manager mocked-HTTP tests (Rust `wiremock`, Python a fake for `requests.request` in `conftest.py`): reads at each use and sees a changed secret, sign-in per read, each status or error code to its fixed text with the manager's words absent, redirect not followed (Rust: the second server receives nothing), dead address, non-text values, Debug shows no setting; the SigV4 signer against AWS's `get-vanilla` vector and a botocore-produced signature (same constants in Rust and Python); Google's assertion verified with the public half of a key generated at test time (no key literal in the repository); the `routes/connectors.rs` unit tests (slot rules, policy, audit details, scheme list); the integration test; bun tests for the helper (against the shared fixture), the form, create, edit and detail pages. Changed, not weakened: `state.rs`'s managed-file test asserts which failure each refusal gives (was "not found"); `test_secret_resolver.py`'s assertions follow the new texts; the store test that `credential_source_of_ref("vault:secret/x")` is `None` now asserts `Reference` and keeps `None` for an unknown scheme (`consul:`); the three stub-fetch tests of the pages gained the settings route.

**Most likely CI failures.**
(a) `g6-secret-managers` (never run): the image tags; the dev-server flags and the `vault status` health check; `moto`'s health URL; the runner's environment (`VAULT_*`, `AWS_*` reach it from `.env` through the override; `AWS_ACCESS_KEY_ID=testing` is also now in the code location's environment during the AWS run, which could change how `dlt`/`pyiceberg` pick S3 credentials); the Dagster run for the wrong-secret step failing with a class the retry rules retry (the gate assumes MySQL "access denied" is final, as `src11` does); timing against the 25-minute limit; `pip install boto3` in the runner.
(b) Rust tests never ran: the integration test (seeded user `bayu@meridian.example` and the Oracle row pattern are copied from `connector_not_testable.rs`), `wiremock` header-matcher case, the botocore constant, the `rsa`/`OsRng` key generation time in debug builds (several seconds per Google test).
(c) `cargo deny`: no new crate, but the `jsonwebtoken` edge moved from dev to normal dependencies (same licence, MIT); `gitleaks`: the CI-only literals (`ci-vault-token-not-real-123`, `testing`).
(d) Any job that writes a `.env` and runs `docker compose` is unaffected (the new variables are `${X:-}`), but a job that runs `docker compose config` in a project where `AWS_REGION` is exported differently in the runner shell is not expected.

**Planner items (B8).** ADR 0002 addendum 6 (the seam, the allowlists, read-at-each-use, the manager's identity as the scope of what a connector can reference: decision 2 above); `docs/OPERATIONS.md` (the settings per manager, the identity's policy, the Google key mount, limits); the feature page; `CHANGELOG.md`; `docs/plans/SRC-12-RESULT.md` part B with "not run against the real services" for Google and Azure and the unrun gate.

### Review B1 fixes (developer)

**Merge.** `git merge origin/feat/phase-1-rest` (commit `807003fd`) merged without conflicts. In `tests/sec11_guard.rs` the `secret_store_error` entry still matches its line (`routes/connectors.rs`, the `ApiError::Unavailable(err.to_string()) // SEC-11 allowlisted` line). The only `{err}` interpolation I had added in routes (`"{}: {err} for {}"` in `slot_changes_with`) is gone: the message is now a fixed sentence; `{failure}` in `resolve_references` is not a name the guard scans. The guard itself was not run (no `cargo test`).

**BLOCKER 1 (`376321e0`).** `SECRET_MANAGER_ALLOWED_PREFIXES`, segment-wise path prefixes (`lakehouse` does not allow `lakehouse-other`), `{tenant}` as a whole segment, scheme-only prefix = allow all of that manager (explicit). Sites: `secret_manager/refs.rs` (`is_clean`, `parse_prefixes`, `reference_allowed`, `TenantMatch`, the two texts); `secret_manager/mod.rs` (`SecretManagerSettings::allowed_prefixes`, `SettingsError::BadPrefixes`, the check in `ManagerResolver::resolve`); `routes/connectors.rs` (`CredentialPolicy` now carries `prefixes` and `tenant`; `slot_changes_with`); `lakehouse-store/src/connectors.rs` (`connector_tenant_id`); `state.rs` (`warn_if_no_reference_prefixes`, once at start-up); `dagster/dispar_orchestrate/secret_manager.py` and `secret_resolver.py` (same rule at use); `src/lib/connectors/secret-reference.ts` (early refusal of `..`, empty, `.`, `%`); compose and `.env.example`; gate `ops/g6/src12_managers_test.py` step 1b and the CI `.env` (`vault:secret/g12/`, switched to `aws-sm:g12/` with the manager). Fixture: `prefix_cases` 28, `prefix_refused` 11, `policy_texts`; read by Rust, Python and bun.
Known limit, said at the sites: the resolver (API at use, orchestrator) does not know the connector, so there `{tenant}` matches any one segment; the exact tenant is checked at save. A row written another way that points into another tenant's subtree is therefore still refused only if it leaves the operator's prefixes altogether. Also: with prefix `<scheme>` alone an operator allows everything; documented in `.env.example`.

**SHOULD-FIX 1 (`106742dd`).** Computed by hand: `ci-vault-token-not-real-123` 3.78 and `ci-vault-token-not-real` 3.53 bits/char, above the generic rule's threshold, so both are allowlisted by exact literal in `.gitleaks.toml` with the reason; `testing` (2.5 bits, 7 characters) is below it and has no entry. Not verified with the scanner.

**SHOULD-FIX 2 (`cd1382fb`).** Comment at `vault-g6` in `ops/g6/docker-compose.src12-managers.override.yml`: the `hashicorp/vault` image is BUSL-1.1 from 1.15 on, used only as a CI test fixture, not distributed; OpenBao (MPL-2.0, API-compatible) is the drop-in if that is ever not acceptable. (The same note is in the Part B handoff above.)

**Commands.** `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` after touching all sources: clean (exit 0, 3 min). `cargo fmt --check`: clean. `pytest dispar_orchestrate -q -m "not loader"`: 1143 passed. `bun run typecheck`: clean; `bun test src/features/connectors src/lib/connectors`: 342 pass, 0 fail. `test_required_gate.sh`: 48 passed. `ops/lint/check_*.py`: ok. YAML (`ci.yml`, `docker-compose.yml`, the override) and TOML (`.gitleaks.toml`) load. **Not run:** `cargo test` (new unit tests in `refs.rs`, `secret_manager/mod.rs`, `routes/connectors.rs`, the integration test `connector_secret_reference.rs` with its new prefix test), docker, the gate, the scanner.

### Fix round after CI fb816a61 (developer)

1. `SRC-12 review B2 BLOCKER 1` (SigV4 vector, `aws.rs:466`): the signer was right, the vector wrong. The botocore constant was made over `json.dumps`' spaced payload `{"SecretId": "prod/x"}`; the Rust test signs serde_json's compact one. Recomputed with botocore (already in the Dagster venv, nothing installed): the Python signer equals botocore for both payloads (spaced `...e1e88`, compact `7e592597...5a6b`). Both clients now send the compact bytes (Python `separators=(",", ":")`), both tests assert the compact constant.
2. `SRC-12 review B2 BLOCKER 2` (`vault.rs:342`, `aws.rs:769`): cause was the tests, not the classification: a dropped `wiremock` MockServer returns to a pool and keeps answering (404 = not found, correctly). New `dead_address()` (bind, take port, drop) in `secret_manager/mod.rs` replaces it in those tests and in the four others meaning "no service" (they passed only because a 404 also meant may-not-sign-in). Assertions unchanged. The Python twins raise `ConnectionError` from the fake and map only a 404 answer to not found: no change.
3. `SRC-12 review B2 BLOCKER 3` (gate step 1b): the `..` reference in the same loop is refused by the grammar, not the prefix rule. Now two checks: a valid reference outside the prefixes must give 400 with the exact prefix text; `..` must give the grammar's "not a valid". The rest of the gate read for the same mistake: none found (not run).

Checks: clippy workspace (all targets, all features, `--locked`, `-D warnings`, after touching all sources) clean; fmt clean; pytest `-m "not loader"` 1143 passed. Not run: cargo test, docker, the gate.

### Part C (developer)

**Commits** (branch `feat/phase-1-src-12c`, cut from `cff86221`, not pushed, in plan order): `ebbbf044` C1 store, dial, routes; `70e114f4` C2 orchestrator; `7e7f06aa` C3 connection test, rotation kept, revoke on sign-out and delete; `f1ee6c83` C4 console; `cfa800f4` C5 gate. This handoff is the commit after. C6 is yours. C1 and C3 were split from one working tree: the C1 state (without the probe, `rotated_refresh_token`, sign-out and delete) was clippy-checked on its own, the final state again.

**Routes** (all in `routes/connector_oauth.rs`; policy entries and `tests/route_auth.rs` in C1, sign-out in C3):

| Method and path | Permission | Request | Response |
| --- | --- | --- | --- |
| `GET /api/connectors/{id}/oauth` | `connector:manage` + tenant gate | none | `{signedIn, callbackUrl, revocationConfigured}`; 409 for a connector that is not authorization code |
| `POST /api/connectors/{id}/oauth/start` | `connector:manage` + tenant gate; a service identity gets 403 | none | `{authorizeUrl, expiresInSeconds}`; 409 when the client secret is not saved or the connector is not authorization code; 503 when `CONNECTOR_OAUTH_REDIRECT_URI` is unset |
| `GET /api/connectors/oauth/callback?code&state[&error]` | `RequiresAuth` (no `{id}`, outside the tenant gate); the handler consumes the state, then needs the user who started it, the connector's tenant and `connector:manage` | the provider's redirect | always an HTML page (200) with a fixed sentence and a nonce-guarded script that closes the window; nothing stored on any refusal |
| `POST /api/connectors/{id}/oauth/refresh-token` | `ingest:read` + `require_orchestrator` (outside the tenant gate) | `{refreshToken}` (`deny_unknown_fields`) | `{stored: true}`; 409 when the connector is not authorization code, 404, 400 |
| `POST /api/connectors/{id}/oauth/sign-out` | `connector:manage` + tenant gate | none | `{signedOut: true, revocation: done\|notConfigured\|noToken\|failed, message}` |
| `GET /api/connectors/credential-settings` (existing) | `connector:manage` | none | gains `oauthCallbackUrl` |

Delete (existing route) revokes before the files go and records `oauthRevocation` in the `connector.delete` audit event. Migration `0080_connector_oauth_state.sql`. New settings: `CONNECTOR_OAUTH_REDIRECT_URI` (operator-set, never from a request), `CONNECTOR_OAUTH_ALLOW_HTTP` (gate-only, default off, exact word `true`; API and `dagster-code-location`); both in `.env.example`.

**Commands run and outcomes.**
- `cargo fmt --check` (workspace): clean.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` (after `touch` of every source, shared target, jobs 2, detached): `Finished`, exit 0, on the final Rust state (`7e7f06aa`, which later commits do not touch); the C1 state (`ebbbf044`) separately, also `Finished`. Earlier runs caught and I fixed: `too_many_lines` in `rotate_secret`, `needless_pass_by_value`, `expect_used` in the store's unit tests, a dead `access_token` field, `mod connector_oauth;` missing in `main.rs` (the bin lists its modules itself).
- `(cd dagster && pytest dispar_orchestrate -q -m "not loader")`: 1196 passed, 258 deselected. New: `test_adapters_rest_oauth.py` (26 test functions), one in `test_ingest_factory.py`, `test_secret_map.py` pin; `test_discover.py`'s REST double follows the new `connector_id` argument.
- `bun run typecheck`: clean. `bun test src/features/connectors src/lib/connectors`: 362 pass, 1 skip, 0 fail. `bun run lint`: 0 errors, the same 6 warnings, none in touched files.
- `bash scripts/ci/tests/test_required_gate.sh`: 50 passed, 0 failed. `ops/lint/check_*.py` (4): ok. YAML loads: `ci.yml`, `docker-compose.yml`, `docker-compose.src12.override.yml`. `py_compile` of the gate. The gate's provider exercised locally by a throwaway script (authorize, token with PKCE, replayed code, refresh rotation, revoke, stale token): as designed.

**Not run.** `cargo test` (any crate): every new Rust test compiles under clippy `--all-targets` and none ran. That is `tests/connector_oauth_flow.rs` (19 tests: sign-in address, status, every callback refusal, good callback sealed, provider refusal, no refresh token, test refreshes and keeps a rotated token, refused refresh, orchestrator report, sign-out, delete, https/private/moved addresses), `lakehouse-store/tests/connector_oauth.rs` (8 `sqlx::test`), `ingest_spec.rs` `oauth_dial_tests` (8), `connector_oauth.rs` unit tests with `wiremock` (12), six probe tests in `connector_probe.rs`, a config test, and the new `route_auth.rs` test. No docker: `docker compose config`/`up` for the two new compose variables, the `g6-oauth` job and `src12_oauth_test.py` never ran against a stack. `cargo deny`, `gitleaks` not run (no new crate, no new key literal; the gate's client secret is generated at run time).

**Tests added or changed.** Added: as above, and console tests for the REST form (five auth types, the new fields, scopes, callback address, slots), the panel (`connector-oauth-panel.test.tsx`, 9), the create flow, the edit flow (client secret alone), the detail page (panel only for OAuth; delete text). Changed, not weakened: `dial-forms.test.tsx` "all four types" became "all five types" (a type was added); `test_discover.py`'s REST fake takes `connector_id`; `rotate_secret`/`test_connection`/`set_credential` call `probe_with` through `run_probe`.

**Decisions of mine to check.**
1. *Slots.* Client secret = primary (kind `secret_key`), refresh token = secondary (kind `token`), `secret_field_names("rest","oauth2_authorization_code") = [clientSecret, refreshToken]`, pinned in `secret_map.py` in the same commit. The client id is configuration in the dial (`auth.clientId`).
2. *Re-pointing a signed-in connector is not possible.* `tokenUrl` and `revocationUrl` are now part of the SEC-14 target identity (also for client credentials, whose `tokenUrl` was not compared before), and a re-point needs every slot the new dial reads, including the refresh token, which can only come from a sign-in. So moving such a connector to another provider host answers 409 and the person must create a new connector. Fail closed, but a limit. The alternative (a re-point drops the refresh token and asks for a new sign-in) needs a change to `repoint_ingest_spec`; your call.
3. *Latent bug fixed on the way.* `RestAuth::Oauth2ClientCredentials { token_url }` only parsed as `token_url` (enum-level `rename_all` renames variants, not their fields), while the console and the adapter use `tokenUrl`; it now parses as `tokenUrl` with `token_url` kept as an alias.
4. *Create is two calls for a sign-in connector.* The create request needs both slots or none, so it carries no values; the client secret is saved right after the connection settings by `PUT .../credential` (probe-first; `verified: false`: nothing to test before a sign-in, `supported: false` with the reason, not a failure).
5. *The verifier is stored in clear in `connector_oauth_state`* for at most 10 minutes, deleted when the state is consumed; only the state is hashed. Sealing it would add a key dependency to a row that lives minutes; say if you want it sealed.
6. *A rotated refresh token and a failed report.* Orchestrator: the new token is held for the rest of the run and a failed report is logged by exception type and the run goes on (the access token in hand is valid; the next run then says "sign in again"). API: a probe that was rotated stores the new token, and a failure to store it is an error to the caller, also when the call that followed failed (`refused_rotation`), except when the rotated token came from a refresh token the request was only proposing (a typed secondary).
7. *Client authentication is `client_secret_post` only.* A provider that accepts only HTTP Basic is not supported; the test says the token request was refused.
8. *The window is opened with `noopener,noreferrer`*, so the callback page cannot message the console; the panel polls (2 s) and asks again on focus. The callback page has a per-response script nonce and a `default-src 'none'` policy.
9. *Every build of a REST source asks for an access token* (one refresh per source object per run), which rotates the refresh token each time on a rotating provider; the next object reads the latest sealed file. Two runs of the same connector at the same moment can race on a rotating provider; not handled.
10. *Platform administrators without a tenant membership* cannot finish a sign-in (the callback uses `connector_in_tenants` like the route layer does).
11. *The expiry margin* is 30 seconds or a fifth of a shorter lifetime (found by the gate's 5-second tokens).
12. *Console: the delete dialog and the form state what the provider can be told* from the dial (`revocationUrl`), without an extra request; the detail page reads the ingest spec to know whether to show the panel.

**Most likely CI failures.**
(a) `g6-oauth`, never run: the runner's address as `src17.own_address("lakehouse-api")` and the API and code location reaching it (the g6 override's `CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS` / `INGEST_ALLOW_INTERNAL_HOSTS`); `CONNECTOR_OAUTH_ALLOW_HTTP` and `CONNECTOR_OAUTH_REDIRECT_URI` reaching both services from `.env` through the new compose lines (not proven by `up`); the session cookie on plain http; Dagster retry/timing around the 8-second loads (`RUN_TIMEOUT_S` 150); the assertion that a load asks for at least two access tokens; the `/managed-secrets` file names (`volume_files_for`, `_token` suffix); the 25-minute limit.
(b) Rust tests never run: `connector_oauth_flow.rs` (wiremock `Respond` closures, the seeded `bayu@meridian.example` and tenant, `PUT .../credential` of a client secret before any sign-in, the SEC-14 conflict test with `127.0.0.2`), the route_auth test's expectation of 409 for the seeded Postgres connector on the three person routes, the store's `sqlx::test`s (`make` of `now() + $5::int * interval`), `sec11_guard` (no `{err}` or `err.to_string()` was added outside logging macros).
(c) `docker compose --profile '*' config --quiet`, `gitleaks`, `cargo deny`: expected clean, not run.

**Planner items (C6).** ADR 0002 addendum 7 (the flow, the state table, the callback's rules, the target identity, `client_secret_post`, the gate-only switch); `docs/OPERATIONS.md` (`CONNECTOR_OAUTH_REDIRECT_URI`, what to enter at the provider, `offline_access`-style scopes, rotation, sign-out and what a missing revocation address means, the re-point limit, concurrent runs); the feature page; `CHANGELOG.md`; `docs/plans/SRC-12-RESULT.md` part C with "not run" for the Rust tests and the gate.

### Fix round after CI dace6134 (developer)

**SRC-12 review C1 BLOCKER 1** (`d67fdc9d`). Cause: `routes/connectors.rs` `ingest_spec_put` applied the SEC-14 re-point rule (`REPOINT_NEEDS_CREDENTIALS`, 409) before any address check, and `tokenUrl` is part of the target identity, so an `http://` token address on a changed dial was reported as a missing credential (test `connector_oauth_flow.rs:933`). Fix: `check_oauth_addresses` (the OAuth part taken out of `check_dial_ssrf`) runs before the re-point rule: `https` (or the gate-only switch) and `resolve_checked` for the dialled addresses, the scheme alone for the sign-in address, 400 with the fixed text. It resolves but sends nothing, so the re-point rule's protection is unchanged. Test assertions kept. Real IPs removed: `93.184.216.34/35/36` became `203.0.113.10/11/12` (RFC 5737; `is_blocked_ip` treats them as public) in `connector_oauth_flow.rs` and `test_adapters_rest_oauth.py`; part C's other addresses are `127.x`, `10.0.0.5`, `169.254.169.254` and `example.com` names. Older files (`test_adapters_rest.py` and others, not part C) still hold `93.184.216.34`; not touched.

**SRC-12 review C1 BLOCKER 2** (`d4089f3a`). Cause: `ops/g6/src12_oauth_test.py` step 4 expected 10 rows, but the source object had no `loadMode`, the default is `replace` (`adapters/sink.py` `LoadPlan`), and the log shows the second run launched and Dagster saw it SUCCEED (`wait_dagster` passed before the row check); 5 rows is correct. The product was not at fault. Fix: the spec sets `loadMode: replace` explicitly; `run_ok` now proves a load by Dagster status, the run record (`GET .../runs`, `status: succeeded`) and 5 rows; step 4 also requires a new refresh-token grant at the provider during the second run and a sealed refresh token that changed again (still a seal, no clear text). The "15 rows" expectation in step 7 went the same way. Read for other such assumptions: step 5 (test passes twice), 6 (wrong and replayed state), 7 (retired token: one attempt `sign_in_refused`), 8 (sign-out and delete revoke the stored token) assert on responses, the provider's counters and the volume, not on rows; none needs changing. `docs/CI.md` follows.

**Checks.** `cargo fmt --check`: clean. `pytest test_adapters_rest_oauth.py`: 30 passed. `py_compile` of the gate. **Not run: clippy and every Rust test** for this round: `free -g` showed 1 GB available for over 40 minutes (a `bun test src` process of someone else holding 6 GB), below the 3 GB limit. The Rust change is one function moved and two call sites with one argument fewer; run clippy before merging. The gate was not run.

## Review

### Reviews, 2026-10-15 (planner)

**Part A.** Accepted as built. `SHOULD-FIX 1`: a credential that cannot be
opened with the key answered the generic "resolved to nothing" — fixed in
part B's B0 with its own fixed text. `BLOCKER` (CI): the image smoke test
started the API without a key — fixed.

**Part B.** `BLOCKER 1` (security): any user with `connector:manage` could
reference any secret the product's manager identity can read and, with a
host of their choosing, carry it off in a connection test. Fixed fail-closed
with `SECRET_MANAGER_ALLOWED_PREFIXES` (segment-wise, `{tenant}`, checked on
save and at every use in the API and the orchestrator; no prefixes, no
references). A stated limit: at use, `{tenant}` matches any one segment
because the resolvers do not know the connector; the exact tenant is checked
at save. CI rounds: a SigV4 test vector recorded over the wrong payload
form; "nobody answers" tests that a pooled mock server answered with 404;
a gate reference invalid by grammar; integration tests that never set the
prefixes; an audit test pinned to the old args. Accepted: the AWS
credential order; references on the re-point route after the `SEC-14`
refusal; create-then-reference in the console; the Vault image is a CI
fixture under BUSL-1.1 (OpenBao is the drop-in).

**Part C.** Accepted, and written as limits: a signed-in connector cannot be
re-pointed; `client_secret_post` only; concurrent runs can race on rotating
providers; the PKCE verifier is kept in clear for at most 10 minutes. CI
rounds: an invalid token address was answered with the re-point refusal
instead of a 400 (order of checks fixed; a real public IP in a test replaced
by a documentation address, rule 12); the gate expected rows to double under
the replace mode.

**`SEC-36`.** This branch's CI-only keys made the full-history `gitleaks`
scan red on `main` and on every PR. Fixed: no key-shaped literal is committed
(CI generates its keys at run time; the test vector describes its key); the
22 historical findings are baselined by exact fingerprint with proof, per
`SEC-1`; `.gitleaks.toml` is back to `main`'s. Both scans pass on #144. The
planner should have caught this in part A's review.

CI on #144 at `daaec3c9` (run 38719076389): all three `SRC-12` gates, every
other gate and both `gitleaks` jobs pass; the last two failures (an
unexported key in the smoke test, a test expecting the settings body without
part C's `oauthCallbackUrl`) are fixed in `bf7a39ad`, whose CI is running.
No open `BLOCKER`. Open for the product owner: the browser check of the
credential form, "Reference a secret" and "Sign in" at the end of the phase,
and the staging secret `STAGING_CONNECTOR_SECRETS_KEY` before the merge.

