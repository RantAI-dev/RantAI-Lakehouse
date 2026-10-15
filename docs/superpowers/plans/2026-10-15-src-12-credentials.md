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

**CI keys.** CI `.env` key = base64 of the 32 ASCII characters `ci-connector-key-not-real-000000` (`Y2ktY29ubmVjdG9yLWtleS1ub3QtcmVhbC0wMDAwMDA=`), in all 14 "Write CI-only .env" steps and in the new job; the job's "new" key = base64 of `ci-connector-key-rotated-0000000`; the API integration tests' key = base64 of 32 bytes of 0x2a (`tests/common/mod.rs`, default for every test app unless overridden). All three plus the vector key are allowlisted in `.gitleaks.toml` by exact literal with the reason (gitleaks not run here; the allowlist is the cautious route). `deploy-staging.yml` now requires repository secret `STAGING_CONNECTOR_SECRETS_KEY` by name.

**Tests added or changed.** New: `secret_crypto` (vector, wrong key, other name, previous key, flipped byte, not-a-seal, key checks), store (start-up pass seals and removes leftovers, unknown-key file left, `reseal_all`, no key means no write, copied file does not open), state (managed file opened, fail closed), Python (vector, previous key, malformed/missing key, plain file refused, end to end), store `credential_source_of_ref`, console detail/edit page text. Changed: store tests now read through the key; `connector_repoint.rs` and `connector_not_testable.rs` read the stored file sealed; `tests/common` gives every test app a key.

**Decisions of mine to check.** (1) Unsealed managed files fail closed in both resolvers, so between a first start with the key and the API's start-up pass an orchestrator read of an old plain file is refused. (2) A sealed file under an unknown key is reported (`unreadable`) and left alone; "looks sealed" = first byte 0x01 and at least 29 bytes. (3) Open failures surface to a user as the existing "resolved to nothing" plus a fixed log line (decision 12 text, ref name only); no new `SecretError` variant. (4) `reseal_all` compares inode, size and mtime just before rename and reports a changed file as failed; a small race remains if a credential is saved during the command, so run it while none is. (5) `docker compose run` of the API passes through `depends_on` of the stack. (6) A6 is a new job (`g6-credentials`, limit 25): `g6-ops` already has limit 30. The g6 override makes `/run/secrets` read-only in the API, so a second override gives the API the writable volume (compose merges `volumes` by mount path; this is the part most likely to need a CI iteration).

**Most likely CI failures.** (a) `g6-credentials`: the volume-merge override, the `run ... g6-ops-test-runner "<command>"` form (entrypoint `/bin/sh -c`), `pip install` of `requests`, the `credential.primary` field in the create response for a managed credential, timing of `up --force-recreate`. (b) Any job I missed that runs `docker compose` without the key now fails at config time with the new `:?` message (all 14 `.env` writers and the staging deploy were updated). (c) `cargo deny`/gitleaks on the new dependency edge and key literals. (d) Rust unit/integration tests never run locally. (e) `dagster` image build if `cryptography==50.0.2` does not resolve for python 3.12-slim.

**Planner items (A7 and docs).** `docs/OPERATIONS.md` has no key-change how-to yet (command in the A4 commit message); the feature page and `CHANGELOG.md`; `STAGING_CONNECTOR_SECRETS_KEY` must exist before the next staging deploy (others' job); the README/quick-start `cp .env.example .env` now needs a generated key.

## Review

*(planner)*
