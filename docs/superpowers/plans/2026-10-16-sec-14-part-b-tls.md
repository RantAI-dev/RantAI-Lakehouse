# `SEC-14` part B: TLS for connector dials, and a test that never sends a password in clear text: plan

Planner: Claude Opus (Security & Delivery lane). Developer: a non-Opus agent.
Base: `origin/main` at `5c93fd20` (after #144). Branch:
`fix/sec-14-part-b-tls`, worktree `/home/shiro/rantai/wt/sec-14b`.
Spec: `docs/core/specs/sec-14.md`. Feature page:
`docs/core/features/connector-repoint-credentials.md` (D1–D10; D5 amended
by the product owner on 2026-10-16, D7, D8 signed; D8's SCRAM is built on
the `postgres-protocol` crate, product owner 2026-10-15). Part A is #85.
The original plan with part B's first design: `docs/superpowers/plans/2026-10-08-sec-14-connector-repoint.md`
§4 tasks 5–8, 11.

## 1. Anchors (verified at `5c93fd20`)

- `rust/crates/lakehouse-api/src/connector_probe.rs`
  - `pg_connect_options` (~1109) and `pg_options_for_address` (~1129):
    the one place the Postgres connect options are built; today
    `.ssl_mode(PgSslMode::Prefer)`. Callers: `dial_postgres` (probe),
    `connector_deprovision.rs` ~280 (`pg_options_for_address`), and
    `connector_discover.rs` (find its Postgres path; it may call
    `pg_connect_options`).
  - `probe_postgres` (~1332) → `dial_postgres`: `resolve_checked` first,
    then the secret, then the dial.
  - SQL Server: `config.trust_cert()` (~558) with a comment pointing at
    part B; `connect_pinned` (~1165) opens TCP to the approved address while
    `config.host` keeps the name for the TLS handshake.
  - `class_of_address_error` (added by #144) matches messages that start
    with "refusing to dial"; keep that prefix wherever you touch messages.
- `rust/crates/lakehouse-api/src/connector_discover.rs` ~545: SQL Server
  `config.trust_cert()` again.
- `rust/crates/lakehouse-api/src/config.rs`: add `CONNECTOR_TLS_CA_FILE`
  next to the other `CONNECTOR_*` settings (`Config::from_map`, README
  table, `.env.example`).
- Crates in `rust/Cargo.lock` today: `tokio-rustls` 0.26, `rustls-pki-types`,
  `webpki-roots`; `tiberius` 0.13 with `rustls`. `postgres-protocol` is
  NOT a dependency yet.
- `docker-compose.yml` service `postgres`; the seeded connector
  `conn-pg-lakehouse` dials host `postgres`.

## 2. Decisions

- E1. One helper builds the Postgres TLS setting for sqlx dials (discovery,
  deprovisioning): `PgSslMode::Require` with no CA;
  `PgSslMode::VerifyCa` plus `ssl_root_cert` when `CONNECTOR_TLS_CA_FILE`
  is set (D5 as amended). Never `Prefer`, never `Disable`.
- E2. The connection test for Postgres stops using sqlx (D8). Its own
  client, in a new module (e.g. `pg_probe.rs`, with a `//!` doc saying why
  it exists and what it refuses):
  1. TCP to the approved address (`connect_pinned`'s pattern), bounded by
     the existing dial timeout.
  2. `SSLRequest`; a server that answers `N` (no TLS) is refused with the
     fixed "TLS error" class, before any startup packet.
  3. TLS with `tokio-rustls`, server name = the configured host name (not
     the IP). With `CONNECTOR_TLS_CA_FILE`: verify the chain against that
     CA and the name (verify-full). Without it: encryption only, using a
     verifier that accepts any certificate, named and documented as such
     (D5's `require`).
  4. Startup message, then authentication: SCRAM-SHA-256 through
     `postgres-protocol`'s `authentication::sasl` (`ScramSha256`) and
     `message::frontend` / `message::backend`. `AuthenticationCleartextPassword`,
     `AuthenticationMD5Password` and any other method are refused with the
     fixed message "server asked for an authentication method this console
     refuses", and no `PasswordMessage` is ever sent for them.
  5. `SELECT 1` (simple query), then terminate.
  Errors map to the existing fixed outcome classes; no upstream text in a
  response (SEC-11). The password lives in `SecretValue` and is exposed
  only into the SCRAM computation.
- E3. SQL Server (both dials): `config.encryption(EncryptionLevel::Required)`
  stated explicitly; with `CONNECTOR_TLS_CA_FILE`, `config.trust_cert_ca(path)`
  instead of `trust_cert()`. Rewrite the two comments that say part B will
  change this.
- E4. Compose (D7). A one-shot init container (`restart: "no"`,
  idempotent: does nothing when the files exist) creates a CA and a server
  certificate for the name `postgres` into a named volume, with the key
  owned by the `postgres` user and mode 0600. The `postgres` service mounts
  it and starts with `ssl=on`, `ssl_cert_file`, `ssl_key_file`; it waits for
  the init container (`service_completed_successfully`). `lakehouse-api`
  mounts the CA read-only and sets `CONNECTOR_TLS_CA_FILE` to it. Other
  clients of this Postgres (Lakekeeper, OpenFGA, Dagster, the API's own
  pool) keep working: `pg_hba.conf` is not changed to require TLS.
- E5. New dependency `postgres-protocol` (MIT or Apache-2.0; `cargo deny`
  must stay green). Pin the minor version. No other new dependency unless
  the plan names it; `rustls-pemfile` or the `rustls-pki-types` PEM API to
  read the CA.
- E6. MySQL is not touched (D9); orchestrator-side TLS is not touched (D10).

## 3. Tasks

Rust tasks first and together; compose after; docs last. One commit each.

**T1. Config.** `CONNECTOR_TLS_CA_FILE` in `config.rs` (path; empty or
unset = none; a set path that cannot be read stops the API at start with a
message naming the variable). README table, `.env.example` (empty, with how
compose sets it). *Check:* config unit tests for unset, set, unreadable.

**T2. sqlx TLS helper (E1).** Used by every sqlx Postgres dial.
*Check:* unit tests: no CA → `Require`; CA → `VerifyCa` with the root
cert; a grep shows no `PgSslMode::Prefer` left in `rust/crates`.

**T3. The probe client (E2).** *Check:* tests with fake servers on a local
listener (no network beyond loopback): (a) server answers `N` to
`SSLRequest` → TLS error class, and the listener receives no startup
packet; (b) server completes TLS then asks `AuthenticationCleartextPassword`
→ the refusal message, and the server never receives a `PasswordMessage`
(this is `SEC-14-AC2`'s regression test; its doc comment says it fails
before the fix); (c) the same for `AuthenticationMD5Password`; (d) a real
SCRAM exchange against the testcontainers Postgres succeeds (it uses
`scram-sha-256` by default); (e) with a CA file, a certificate for another
name is refused. Mutation evidence: make (b) send the password and show
the test fail, then restore; quote both.

**T4. SQL Server (E3).** *Check:* unit test on the built `Config`
(encryption required; CA used when set).

**T5. Compose (E4).** *Check:* `docker compose --profile '*' config --quiet`
with an `ops/init-env.sh` `.env`; then a real `up` (rule 8) of the default
profile under the heavy lock, project `sec14b`, ports remapped, `down -v`
via `trap`: the init container exits 0; `psql` from inside `postgres`
shows `SHOW ssl` = `on`; the seeded connector's test
(`POST /api/connectors/conn-pg-lakehouse/test`, signed in as the bootstrap
admin) succeeds over verify-full, and `pg_stat_ssl` shows the connection
encrypted; a re-run of `up` on the existing volume leaves the certificate
unchanged (idempotent).

**T6. Docs.** `CHANGELOG.md` (Security); `docs/OPERATIONS.md`: what
`CONNECTOR_TLS_CA_FILE` does, how compose sets it, what an operator with
their own Postgres does; `docs/core/specs/sec-14.md` "Today" cells for TLS
corrected; the two `connector_probe.rs`/`connector_discover.rs` comments.

## 4. Verification before handoff

`cargo fmt --check`; workspace clippy `-D warnings`; `cargo test -p
lakehouse-api --no-fail-fast` (quote every binary's `test result:`; rerun
an unrelated failure alone with `RUST_TEST_THREADS=2` before calling it
flaky); `cargo deny check` if installed, else say *not verified* (CI runs
it); compose as T5; gitleaks `dir` (ignore only the gitignored `.env`).

## 5. Pull request

One slice: T2–T4 without T5 would make the seeded connector's test fail on
a compose stack, so they ship together.

## 6. Out of scope

MySQL (D9); orchestrator-side TLS (D10); requiring TLS for the stack's own
clients (`pg_hba`); `SEC-25` (separate plan, same files: do not touch its
lines).

## 7. Verify, do not assume

- Does `postgres-protocol`'s current release expose `ScramSha256` with the
  API this plan assumes? Read its docs at the pinned version.
- Does `tiberius` 0.13 have `trust_cert_ca`? If not, stop and report.
- Does the postgres image accept a key owned by `postgres` mode 0600 from a
  named volume populated by another container? Prove it in T5.

## 8. Handoff

### Commits on branch `fix/sec-14-part-b-tls`
- `832df0fc`: feat(config): CONNECTOR_TLS_CA_FILE for connector dials (SEC-14 part B)
- `4dfbd687`: feat(connectors): require TLS for every sqlx Postgres dial (SEC-14 part B)
- `75a192de`: feat(connectors): Postgres connection test speaks the protocol itself, SCRAM-SHA-256 only (SEC-14 part B)
- `bbe40ee8`: feat(connectors): require TLS for SQL Server dials and verify CA when set (SEC-14 part B)
- `053eacb8`: feat(compose): TLS for bundled postgres and verify-full connector probe (SEC-14 part B)
- `7638a4a5`: docs: document CONNECTOR_TLS_CA_FILE and compose TLS posture (SEC-14 part B)
- `78cb75de`: test(connectors): route integration probes through TlsProxy for TLS requirement (SEC-14 part B)

### Verification Summary

1. **`cargo fmt --check`**
   - Command: `cd rust && cargo fmt --check`
   - Output: clean exit (0 diffs).

2. **Workspace Clippy (`-D warnings`)**
   - Command: `cd rust && CARGO_TARGET_DIR=/home/shiro/rantai/cargo-target flock /home/shiro/rantai/.locks/heavy.lock cargo clippy -j 4 -p lakehouse-api --all-targets -- -D warnings`
   - Output:
     ```
         Checking lakehouse-api v0.1.0 (/home/shiro/rantai/wt/sec-14b/rust/crates/lakehouse-api)
         Finished `dev` profile [unoptimized + debuginfo] target(s) in 1m 22s
     ```
     0 errors, 0 warnings.

3. **`cargo test -j 4 -p lakehouse-api --no-fail-fast`**
   - Command: `cd rust && CARGO_TARGET_DIR=/home/shiro/rantai/cargo-target RUST_TEST_THREADS=2 flock /home/shiro/rantai/.locks/heavy.lock cargo test -j 4 -p lakehouse-api --no-fail-fast`
   - Verbatim `test result:` lines:
     - `src/lib.rs`: `test result: ok. 1927 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 308.66s`
     - `src/main.rs`: `test result: ok. 1973 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 284.58s`
     - `tests/agents_approval.rs`: `test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 8.67s`
     - `tests/agents_run.rs`: `test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 12.90s`
     - `tests/ai_ask_back_turn.rs`: `test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.54s`
     - `tests/ai_chart_draft.rs`: `test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.87s`
     - `tests/ai_chat_stream.rs`: `test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.87s`
     - `tests/ai_citations.rs`: `test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.26s`
     - `tests/ai_sql_help.rs`: `test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 9.39s`
     - `tests/ai_terms.rs`: `test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.95s`
     - `tests/ai_tool.rs`: `test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 9.00s`
     - `tests/connector_delete_deprovision.rs`: `test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 14.18s`
     - `tests/connector_discovery_launch.rs`: `test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.97s`
     - `tests/connector_discovery_routes.rs`: `test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 7.00s`
     - `tests/connector_not_testable.rs`: `test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.02s`
     - `tests/connector_oauth_flow.rs`: `test result: ok. 20 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 33.01s`
     - `tests/connector_probe_history_route.rs`: `test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.07s`
     - `tests/connector_repoint.rs`: `test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 18.41s`
     - `tests/connector_secret_reference.rs`: `test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 8.39s`
     - `tests/connector_secret_rotate.rs`: `test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 11.80s`
     - `tests/connector_tenant_access.rs`: `test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 12.41s`
     - `tests/connector_upload_table.rs`: `test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 7.37s`
     - `tests/dashboard_export.rs`: `test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 6.69s`
     - `tests/debezium_properties.rs`: `test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 7.00s`
     - `tests/embed_tokens.rs`: `test result: ok. 22 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 30.33s`
     - `tests/explain_rewrite.rs`: `test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 6.83s`
     - `tests/gold_export_auth.rs`: `test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.78s`
     - `tests/gold_publication.rs`: `test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 27.26s`
     - `tests/home_layout.rs`: `test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.34s`
     - `tests/identity_user_tenants.rs`: `test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.07s`
     - `tests/ingest_run.rs`: `test result: ok. 22 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 34.75s`
     - `tests/lakehouse_maintenance.rs`: `test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 11.62s`
     - `tests/login_throttle.rs`: `test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 14.00s`
     - `tests/parity.rs`: `test result: ok. 0 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 0.00s`
     - `tests/pipeline_enforce.rs`: `test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 14.20s`
     - `tests/pipeline_routing.rs`: `test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.48s`
     - `tests/query_author_exception.rs`: `test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.82s`
     - `tests/query_download.rs`: `test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 14.67s`
     - `tests/reporting_settings_route.rs`: `test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 7.64s`
     - `tests/route_auth.rs`: `test result: ok. 40 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 43.16s`
     - `tests/sec11_guard.rs`: `test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.39s`
     - `tests/security_regressions.rs`: `test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 14.77s`
     - `tests/semantic_api.rs`: `test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 17.50s`
     - `tests/tenant_provisioning.rs`: `test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 9.38s`
     - `tests/test_connection_route.rs`: `test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.48s`
     - `tests/tier1_saved_query_guard.rs`: `test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.09s`
     - `tests/tier1_writehigh_approval.rs`: `test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 7.82s`
     - `tests/tier2_governance_drafts.rs`: `test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 6.19s`
     - `tests/tier2_writehigh_approval.rs`: `test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.43s`
     - `tests/upload_routes.rs`: `test result: ok. 88 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 128.72s`
     - `tests/upstream_errors.rs`: `test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.46s`
     - `Doc-tests lakehouse_api`: `test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s`

4. **Mutation Evidence for T3(b)**
   - Mutated `src/pg_probe.rs` to write `password_message(password.as_bytes(), &mut out)` when `Message::AuthenticationCleartextPassword` was received from server.
   - Test `pg_probe::tests::cleartext_password_is_refused_and_no_password_is_sent` failed as expected:
     ```
     thread 'pg_probe::tests::cleartext_password_is_refused_and_no_password_is_sent' panicked at crates/lakehouse-api/src/pg_probe.rs:583:13:
     no PasswordMessage may follow a cleartext request: [112, 0, 0, 0, 7, 112, 119, 0]
     test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 1927 filtered out; finished in 0.05s
     ```
   - Restored `src/pg_probe.rs` to pristine HEAD. Test passed:
     ```
     test pg_probe::tests::cleartext_password_is_refused_and_no_password_is_sent ... ok
     test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1927 filtered out; finished in 0.05s
     ```

5. **T5 Compose Drill Evidence (`sec14b-t5.log`)**
   - Verified `docker compose --profile '*' config --quiet` passed cleanly with `.env` from `ops/init-env.sh`.
   - Real `up` of default profile with project `sec14b` under heavy lock:
     - `postgres-cert-init` generated self-signed root CA and `postgres` server certificate/key with ownership 999:999 mode 0600, exiting 0.
     - `psql` execution inside `postgres` confirmed:
       ```
        ssl 
       -----
        on
       (1 row)

        ssl | version |         cipher         | bits 
       -----+---------+------------------------+------
        t   | TLSv1.3 | TLS_AES_256_GCM_SHA384 |  256
       (1 row)
       ```
     - Seeded connector probe `POST /api/connectors/conn-pg-lakehouse/test` (authenticated as bootstrap admin) verified over verify-full:
       ```json
       {"ok":true,"supported":true,"latencyMs":27,"message":"Connected via PostgreSQL and ran SELECT 1.","testedAt":"2026-10-16T11:36:26.280Z"}
       ```
     - `pg_stat_ssl` confirmed active connection encrypted via `TLSv1.3` / `TLS_AES_256_GCM_SHA384`.
     - Re-running `up` on existing volume left certificates identical (idempotent):
       `server.crt`: `e9e69801c0dd896b3d9a326673004104`
       `ca.crt`: `988ed601b2dc3b88aff7d83a3e96f0dd`
       `server.key`: `0d7acb0d22a64f31ab170da8cd564475`
     - Teardown cleanly cleaned all containers and volumes via `docker compose -p sec14b down -v`.

6. **`cargo deny check`**
   - Command: `cd rust && cargo deny check`
   - Output: `advisories ok, bans ok, licenses ok, sources ok` (exit 0).

7. **Gitleaks Scan**
   - Command: `gitleaks git --log-opts="origin/main..HEAD" -v`
   - Output: `8 commits scanned`, `no leaks found` (exit 0).

### Handoff (round 2)

#### Commits on branch `fix/sec-14-part-b-tls` (round 2)
- `2db17d61`: fix(compose): restrict CA mount for API container to public certificate alone (SEC-14 part B) [BLOCKER 1]
- `1f86647e`: fix(connectors): cap message length in pg_probe to prevent buffer expansion (SEC-14 part B) [BLOCKER 2]
- `61b5c28d`: fix(connectors): require completed SCRAM exchange and reject unauthenticated Postgres connections (SEC-14 part B) [BLOCKER 3]
- `ae482249`: fix(config): validate PEM certificate presence in CONNECTOR_TLS_CA_FILE at startup (SEC-14 part B) [SHOULD-FIX 6]
- `4ecafbf9`: fix(deps): pin postgres-protocol to 0.6.12 for SCRAM iteration cap (SEC-14 part B) [SHOULD-FIX 9]
- `5dcdc2dc`: docs(connectors): correct TLS channel binding, CA bundling, Tiberius roots, and changelog names (SEC-14 part B) [SHOULD-FIX 4, 5, 7, 8]
- `7c6f6644`: docs(connector_probe): add backticks to PostgreSQL in doc comment [SHOULD-FIX 7]

#### Findings Addressed & Verification Evidence

1. **BLOCKER 1: Restrict CA mount in API container to public certificate alone**
   - In `docker-compose.yml`, `postgres-cert-init` removes `ca.key` and `ca.srl` immediately after signing `server.crt`.
   - `lakehouse_postgres_ca` volume was created to hold `ca.crt` alone, mounted read-only into `lakehouse-api` at `/run/certs/postgres:ro`.
   - Re-run compose drill verified:
     - `lakehouse-api` sees only `ca.crt` in `/run/certs/postgres`:
       ```
       total 8
       drwxr-xr-x 2 root root 4096 Oct 16 14:48 .
       drwxr-xr-x 3 root root 4096 Oct 16 14:48 ..
       -rw-r--r-- 1 root root 1155 Oct 16 14:48 ca.crt
       ```
     - Neither `ca.key` nor `ca.srl` exists in any volume after initialization.
     - Seeded connector probe `POST /api/connectors/conn-pg-lakehouse/test` passes over `verify-full`:
       `{"ok":true,"supported":true,"latencyMs":6,"message":"Connected via PostgreSQL and ran SELECT 1.","testedAt":"2026-10-16T14:48:47.457Z"}`
     - Idempotency verified: re-running init on existing volumes preserves certificates unchanged.

2. **BLOCKER 2: Prevent buffer expansion in pg_probe**
   - Added `const MAX_BACKEND_MESSAGE_LEN: usize = 64 * 1024` in `rust/crates/lakehouse-api/src/pg_probe.rs`.
   - `read_message` inspects the 5-byte header before `Message::parse` and refuses messages with length > 64 KiB or < 4 as `PgProbeError::Protocol`.
   - **Mutation test**:
     - Removed the check in `read_message`.
     - Test `pg_probe::tests::oversized_backend_message_length_is_refused_without_buffer_growth` failed with:
       ```
       thread 'pg_probe::tests::oversized_backend_message_length_is_refused_without_buffer_growth' panicked at crates/lakehouse-api/src/pg_probe.rs:857:9:
       assertion failed: matches!(err, PgProbeError::Protocol)
       test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 1929 filtered out; finished in 0.00s
       ```
     - Restored fix; test passed:
       ```
       test pg_probe::tests::oversized_backend_message_length_is_refused_without_buffer_growth ... ok
       test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1929 filtered out; finished in 0.00s
       ```

3. **BLOCKER 3: Enforce completed SCRAM exchange and reject unauthenticated connections**
   - Added `scram_finished` tracking in `authenticate_and_select`. If `AuthenticationOk` arrives before `scram_finished`:
     - If SCRAM was in progress, returns `Err(PgProbeError::Authentication)`.
     - Otherwise returns `Err(PgProbeError::NoPasswordAsked)` with fixed message `"the server did not ask for a password"`.
   - **Mutation test**:
     - Reverted `AuthenticationOk` handling to unconditional `break`.
     - Tests failed with:
       ```
       thread 'pg_probe::tests::scram_skipped_final_signature_is_refused' panicked at crates/lakehouse-api/src/pg_probe.rs:935:9:
       assertion `left == right` failed
         left: "connection failed"
        right: "authentication failed"
       test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 1931 filtered out; finished in 0.11s

       thread 'pg_probe::tests::trust_server_is_refused_without_reporting_credential_verified' panicked at crates/lakehouse-api/src/pg_probe.rs:981:10:
       trust server must be refused: ()
       test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 1931 filtered out; finished in 0.05s
       ```
     - Restored fix; tests passed:
       ```
       test pg_probe::tests::scram_skipped_final_signature_is_refused ... ok
       test pg_probe::tests::trust_server_is_refused_without_reporting_credential_verified ... ok
       test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 1930 filtered out; finished in 0.16s
       ```

4. **SHOULD-FIX 6: Validate PEM certificate presence at startup**
   - In `rust/crates/lakehouse-api/src/config.rs`, `parse_connector_tls_ca_file` iterates PEM certificates via `rustls::pki_types::CertificateDer::pem_file_iter`.
   - If the file has 0 valid certificates or cannot be parsed, fails startup with `ConfigError::EmptyConnectorTlsCaFile(path)` naming `CONNECTOR_TLS_CA_FILE`.
   - Unit tests `an_empty_connector_tls_ca_file_stops_the_api` and `a_connector_tls_ca_file_is_absent_by_default_and_kept_when_readable` verify this behavior.

5. **SHOULD-FIX 9: Pin postgres-protocol version**
   - In `rust/Cargo.toml`, pinned `postgres-protocol = "0.6.12"` with commentary documenting reliance on its SCRAM iteration limit.

6. **SHOULD-FIX 4, 5, 7, 8: Documentation and comment corrections**
   - Corrected channel binding comment in `pg_probe.rs` and documented in `docs/OPERATIONS.md` that without a CA a relaying server is not detected (`channel_binding=disable`).
   - Documented in `docs/OPERATIONS.md` and `.env.example` that Compose falls back to `/run/certs/postgres/ca.crt` when empty, requiring external database operators to mount a combined CA bundle.
   - Documented in `connector_probe.rs` and `docs/OPERATIONS.md` that Tiberius `trust_cert_ca` adds the CA to native roots (accepting either the CA or public roots), unlike PostgreSQL.
   - Fixed function and helper names in `CHANGELOG.md` (`connector_deprovision::drop_slot_and_publication`, table listing helper `connector_listing`).

#### Final Checks Summary

1. **`cargo fmt --check`**: Clean (exit code 0).
2. **Workspace clippy (`-D warnings`)**:
   - `cargo clippy --workspace --all-targets -j 4 -- -D warnings`: 0 warnings, 0 errors (exit code 0).
3. **`cargo test -j 4 -p lakehouse-api --no-fail-fast`**:
   - `src/lib.rs`: `test result: ok. 1932 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 330.87s`
   - `src/main.rs`: `test result: ok. 1978 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 321.98s`
   - `tests/agents_approval.rs`: `test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 16.54s`
   - `tests/agents_run.rs`: `test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 22.20s`
   - `tests/ai_ask_back_turn.rs`: `test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 13.23s`
   - `tests/ai_chart_draft.rs`: `test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 20.85s`
   - `tests/ai_chat_stream.rs`: `test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.04s`
   - `tests/ai_citations.rs`: `test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.79s`
   - `tests/ai_sql_help.rs`: `test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 52.45s`
   - `tests/ai_terms.rs`: `test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 13.25s`
   - `tests/ai_tool.rs`: `test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 7.64s`
   - `tests/connector_delete_deprovision.rs`: `test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 35.79s`
   - `tests/connector_discovery_launch.rs`: `test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.42s`
   - `tests/connector_discovery_routes.rs`: `test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.55s`
   - `tests/connector_ingestible_tenant.rs`: `test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 8.04s`
   - `tests/connector_not_testable.rs`: `test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.10s`
   - `tests/connector_oauth_flow.rs`: `test result: ok. 20 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 26.06s`
   - `tests/connector_probe_history_route.rs`: `test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.40s`
   - `tests/connector_repoint.rs`: `test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 22.60s`
   - `tests/connector_secret_reference.rs`: `test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 14.76s`
   - `tests/connector_secret_rotate.rs`: `test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 12.16s`
   - `tests/connector_tenant_access.rs`: `test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 12.64s`
   - `tests/connector_upload_table.rs`: `test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.91s`
   - `tests/dashboard_export.rs`: `test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.79s`
   - `tests/debezium_properties.rs`: `test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.44s`
   - `tests/embed_tokens.rs`: `test result: ok. 22 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 28.85s`
   - `tests/explain_rewrite.rs`: `test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 9.55s`
   - `tests/gold_export_auth.rs`: `test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.71s`
   - `tests/gold_publication.rs`: `test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 58.75s`
   - `tests/home_layout.rs`: `test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.41s`
   - `tests/identity_user_tenants.rs`: `test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 11.88s`
   - `tests/ingest_run.rs`: `test result: ok. 22 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 37.51s`
   - `tests/lakehouse_maintenance.rs`: `test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 8.74s`
   - `tests/login_throttle.rs`: `test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 23.34s`
   - `tests/parity.rs`: `test result: ok. 0 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 0.00s`
   - `tests/pipeline_enforce.rs`: `test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 16.02s`
   - `tests/pipeline_routing.rs`: `test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.77s`
   - `tests/query_author_exception.rs`: `test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 9.75s`
   - `tests/query_download.rs`: `test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 23.41s`
   - `tests/reporting_settings_route.rs`: `test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 11.76s`
   - `tests/route_auth.rs`: `test result: ok. 40 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 56.31s`
   - `tests/sec11_guard.rs`: `test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.42s`
   - `tests/security_regressions.rs`: `test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 11.74s`
   - `tests/semantic_api.rs`: `test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 29.04s`
   - `tests/tenant_provisioning.rs`: `test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 43.15s`
   - `tests/test_connection_route.rs`: `test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.88s`
   - `tests/tier1_saved_query_guard.rs`: `test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 7.17s`
   - `tests/tier1_writehigh_approval.rs`: `test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 9.43s`
   - `tests/tier2_governance_drafts.rs`: `test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.34s`
   - `tests/tier2_writehigh_approval.rs`: `test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.28s`
   - `tests/upload_routes.rs`: `test result: ok. 88 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 178.74s`
   - `tests/upstream_errors.rs`: `test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 9.29s`
   - `Doc-tests lakehouse_api`: `test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s`
4. **`cargo deny check`**:
   ```
   advisories ok, bans ok, licenses ok, sources ok
   ```
5. **Gitleaks Scan**:
   ```
   4:56PM INF scanned ~22362916 bytes (22.36 MB) in 4.82s
   4:56PM INF no leaks found
   ```

## 9. Review

Round 1, reviewed 2026-10-16 at `10e2b8fe` (developers: DeepSeek, MiniMax
M3.1, Gemini; read with a separate security reviewer). E1, E2's refusal
paths, E3, E5, E6 are implemented as planned; T3(b)'s mutation evidence
and T5's compose run (TLSv1.3, the seeded connector's test over
verify-full) are real (`review-logs/sec14b-t5.log`). `tls_proxy.rs` is
`#[cfg(test)]` only: fine.

**BLOCKER 1. The CA's private key reaches the API container.** The
`postgres-cert-init` step writes `ca.key` (and `ca.srl`, `server.key`) into
`lakehouse_postgres_tls` (`docker-compose.yml` ~35–48; the cleanup removes
only `server.csr`, `server.ext`), and `lakehouse-api` mounts the whole
volume (~657). The API image runs as root (`rust/Dockerfile` has no
`USER`), so it can read the CA signing key, and that CA is the only trust
root for every connector dial: anyone in the API container could mint a
certificate for any database name and pass verify-full. E4 says the API
mounts *the CA certificate*. Fix: delete `ca.key` and `ca.srl` after
signing (nothing needs them afterwards; the idempotence check uses
`ca.crt`, `server.crt`, `server.key`), and give the API only `ca.crt` (a
second volume, or a copy written into a separate volume holding the
certificate alone). Prove in T5's re-run that the API container has no
file other than `ca.crt` under its mount.

**BLOCKER 2. A server can make the probe reserve about 4 GiB.**
`pg_probe.rs` `read_message` (~424–440) calls `Message::parse` with no
length limit; `postgres-protocol` reads the 4-byte length from the header
and reserves that much. Fix: read the 5-byte header yourself, refuse a
length above a small cap (64 KiB is plenty for every message the probe
expects) as `Protocol` before parsing, and add a fake-server test that
sends `R` with a length of `0xFFFFFFFF` and is refused without growing the
buffer.

**BLOCKER 3. The server's SCRAM signature check can be skipped.**
`pg_probe.rs` ~315 accepts `AuthenticationOk` in any state: a server that
answers `SaslContinue` and then `AuthenticationOk` without `SaslFinal`
never has `finish` run, and the test reports success. Once SCRAM has
started, `AuthenticationOk` before a successful `finish` is a refusal. And
`AuthenticationOk` with no authentication at all (a `trust` server) must
not report the credential as verified: refuse it with a fixed message
("the server did not ask for a password") unless the plan's owner says
otherwise. Tests for both.

**SHOULD-FIX 4.** The comment at ~320–322 says sending `n,,` (no channel
binding) is what libpq sends; libpq's default `channel_binding=prefer`
uses `SCRAM-SHA-256-PLUS` when offered over TLS. Correct the comment.
Implementing `tls-server-end-point` binding is optional; if not done, say
in `docs/OPERATIONS.md` that without a CA a relaying server is not
detected.

**SHOULD-FIX 5.** `docker-compose.yml` ~388
`${CONNECTOR_TLS_CA_FILE:-/run/certs/postgres/ca.crt}` cannot be cleared
from `.env`, while `docs/OPERATIONS.md` ~730 tells operators with publicly
signed databases to "leave it empty". Document what an operator does
instead (a bundle that includes the bundled CA and their own roots), in
OPERATIONS and `.env.example`.

**SHOULD-FIX 6.** `config.rs` ~1163 only checks the CA file is readable;
also parse at least one PEM certificate at startup (a file with none stops
the API with a message naming the variable), as `pg_probe.rs` ~192 says.

**SHOULD-FIX 7.** tiberius' `trust_cert_ca` adds the CA to the native
roots, so SQL Server accepts the configured CA or any public root, unlike
Postgres. State that difference at `connector_probe.rs` ~1190 and in
OPERATIONS ~714.

**SHOULD-FIX 8.** `CHANGELOG.md` ~244–245 names
`connector_probe::drop_cdc_publication_and_slot` (it is
`connector_deprovision::drop_slot_and_publication`) and calls the third
SQL Server dial a "CDC drop helper" (it is `connector_listing`).

**SHOULD-FIX 9.** Pin `postgres-protocol = "0.6.12"` in `rust/Cargo.toml`
with the reason (the SCRAM iteration cap this code relies on is there).

Round 2, reviewed 2026-10-17 at `1668cbe2`. All three BLOCKERs and
SHOULD-FIX 4–9 are fixed, each cited at its fix site:

- BLOCKER 1 (`2db17d61`): `ca.key` and `ca.srl` are deleted after signing;
  `ca.crt` is copied into a second volume, `lakehouse_postgres_ca`, which is
  the only one `lakehouse-api` mounts. The compose re-run in the handoff
  lists `ca.crt` alone under the API's mount.
- BLOCKER 2 (`1f86647e`): `read_message` checks the 5-byte header and
  refuses a length below 4 or above 64 KiB (`MAX_BACKEND_MESSAGE_LEN`) as
  `Protocol` before `Message::parse`; a fake-server test sends
  `0xFFFFFFFF`.
- BLOCKER 3 (`61b5c28d`): `AuthenticationOk` after `SaslContinue` but
  before a verified `SaslFinal` is refused, and a server that never asks
  for a password is refused as `NoPasswordAsked` ("the server did not ask
  for a password"). Mutation evidence for each is in the round-2 handoff.
- SHOULD-FIX 4–9: comment corrected (no channel binding, the relay risk
  without a CA documented in OPERATIONS); the `:-` CA default documented
  with the combined-bundle route in OPERATIONS and `.env.example`; startup
  parses at least one PEM certificate; tiberius' extra public roots stated
  at `connector_probe.rs` and in OPERATIONS; changelog names corrected;
  `postgres-protocol = "0.6.12"` pinned with the reason.

Planner verification at `1668cbe2`, run from the shared target under the
heavy lock (`review-logs/sec14b-review2.log`): `cargo fmt --check` ok;
`cargo clippy --workspace --all-targets --all-features -- -D warnings`
exit 0; `cargo deny check` exit 0; `RUST_TEST_THREADS=2 cargo test -p
lakehouse-api -p lakehouse-store --no-fail-fast`: 5009 passed, 29 failed,
4 ignored. The 29 failures were in three binaries (`connector_oauth_flow`
17, `connector_probe_history_route` 4, `connector_repoint` 8) while the
machine's load average was 15–30 from other stacks; rerun alone with
`RUST_TEST_THREADS=1` (`review-logs/sec14b-rerun.log`) all three pass:
20, 5 and 13 passed, 0 failed. Not a regression.

No open BLOCKER. Ready for the pull request.

Round 3, 2026-10-17, from CI on PR #192 at `a5c46e83` (main is green on
the same gates, so these are this PR's). Three gates fail: G4 Debezium
CDC, G6 ingest matrix (SQL sources), SRC-13 discovery of 10,000 tables.

**BLOCKER 10. Every TLS dial that does not name a provider panics the
API.** The API log in G4, G6 and SRC-13 shows `thread 'tokio-rt-worker'
panicked at rustls-0.23.46/src/crypto/mod.rs:249: Could not automatically
determine the process-level CryptoProvider from Rustls crate features`.
`cargo tree -e features -i rustls` shows both `ring` and `aws-lc-rs`
enabled in `lakehouse-api` (`aws-lc-rs` arrives through `object_store`,
`hyper-rustls` and `tokio-rustls`'s default features), and nothing calls
`CryptoProvider::install_default`. Before this PR `Prefer` fell back to
plaintext against a server without `ssl`, so sqlx's rustls path never
ran; with `ssl=on` and `Require`/`VerifyCa` it runs on the first
discovery and the handler panics (the client sees the connection closed).
`pg_probe` survives only because it builds with an explicit provider.
Fix:
- install one provider (`ring`, matching `pg_probe` and the workspace
  pin) as the process default at the very start of `main`, before any
  task that can dial; refuse to start (log and exit non-zero, no
  `unwrap`) if installation fails for a reason other than "already
  installed";
- correct the `rust/Cargo.toml` comment (~104–105) that says everything
  uses `ring` only: say both features are present and why the default is
  installed;
- add a test that dials through the production sqlx connect-options
  helper (`Require` and `VerifyCa`) against `tls_proxy` and passes, with
  the test binary calling the same install function `main` calls; give
  the mutation evidence (remove the install, the test panics with the
  message above);
- check every other rustls user in the API that relies on the process
  default (sqlx, tiberius, `reqwest`, `lettre`) is covered by the same
  install.

**BLOCKER 11. G6's SQL Server connect fails with `TLS error`.**
`ops/g6/docker-compose.g6.override.yml` gives `mssql-g6` its own
self-signed certificate (`mssql-g6-cert-init`, volume `mssql_g6_tls`);
the API now trusts `CONNECTOR_TLS_CA_FILE`, the bundled Postgres CA, so
tiberius rejects `mssql-g6`. The refusal is right (fail closed); the gate
is what has to change, and it should use the route OPERATIONS documents:
a combined bundle. In the G6 override only (gate-only, commented like the
other GATE-ONLY entries): an init step writes the Postgres `ca.crt` plus
`mssql-g6.pem` into a gate volume, and `lakehouse-api` points
`CONNECTOR_TLS_CA_FILE` at it. Do not set `trust_cert`, do not loosen
`EncryptionLevel`. Confirm `mssql-g6`'s certificate names the host the
gate dials, first ruling out BLOCKER 10 as the cause (tiberius also uses
rustls).

Proof required in the handoff (AGENTS.md rule 8): run G4, G6
(`g6-sql` half) and SRC-13 locally from a clean project, the same
commands as `.github/workflows/ci.yml`, and quote each gate's result
line. One gate at a time, under the heavy lock, `docker compose down -v`
on your own project between them.
