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

(developer)

## 9. Review

(planner)
