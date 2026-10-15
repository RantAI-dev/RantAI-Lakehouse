# `SRC-12` Credentials done properly — Implementation Plan

**Status:** ready to build, part A first. The product owner signed the
feature page's decisions on 2026-10-14 ("setuju semua"). Written 2026-10-15
by the planner (Claude Opus) for developer agents, under the role split in
`AGENTS.md`. Size L: three parts (decision 13), each with its own gate and
review before the next starts.

**Base and branch:** the Phase 1 branch `feat/phase-1-rest` (draft PR #144).
Built in its own worktree on a sub-branch cut from the phase branch and
merged back into it, so it can run beside `SRC-13`. Migration numbers
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

## Review

*(planner)*
