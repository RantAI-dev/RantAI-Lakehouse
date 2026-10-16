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

## 9. Review

(planner)
