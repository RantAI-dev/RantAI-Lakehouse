# `SEC-14` Connector passwords cannot be stolen by re-pointing a connector — Implementation Plan

**Status:** ready to build. On 2026-10-08 the product owner asked for all
five Phase 0 security items to be fixed as their specs say; the decisions
in section 1 marked *(proposed)* are taken as proposed on that instruction
and are recorded on the feature page. Written 2026-10-08 by the planner
(Claude Opus) for a developer agent, under the role split in `AGENTS.md`.

**Spec:** `docs/core/specs/sec-14.md`. Backlog `SEC-14`.
**Feature page:** `docs/core/features/connector-repoint-credentials.md`.

**Base:** branch `fix/sec-14-connector-repoint`, created from
`fix/sec-15-probe-internal-addresses` (pull request #82), because both
change the same dials and `SEC-15` was built first. Section 6 below said
the opposite order; it is superseded. The helpers `SEC-15` added
(`pg_connect_options`, `mysql_connect_options`, `connect_pinned`, the
`Approved` address) are where tasks 5 to 7 now make their changes.

**Disclosure.** The spec describing this path is already on `main`
(`docs/core/specs/sec-14.md`), so the fix branch discloses nothing new.

**Why.** A user with `connector:manage` can change where a connector
points while it keeps its stored credential. The next test, discovery,
delete or scheduled ingest sends that credential to the new server. The
seeded connector's credential is the console's own database password
(`docker-compose.yml:297`).

---

## 1. Decisions

| # | Decision | Choice | Signed |
| --- | --- | --- | --- |
| D1 | What "re-pointing" means | A change to the connector's *target identity* (table in §3). | *(proposed)* |
| D2 | Response without credentials | `409` with a fixed message, nothing written, no dial made. From the spec. | spec |
| D3 | Which slots must be re-sent | Every slot the connector has after the change. One slot of two is still `409`. | *(proposed)* |
| D4 | `PATCH /api/connectors/{id}` `host` | Same rule as D1. Not in the spec's Target table; see finding F3. | *(proposed)* — needs owner |
| D5 | PostgreSQL TLS floor | `require`; `verify-full` when a CA file is configured. From the spec. | spec |
| D6 | Where the CA comes from | One deployment setting, `CONNECTOR_TLS_CA_FILE` (`${X:-}`), used for PostgreSQL and SQL Server. | *(proposed)* |
| D7 | Compose PostgreSQL has no TLS | Turn TLS on for the compose PostgreSQL and mount its CA into the API. No plaintext opt-out. See finding F6. | *(proposed)* — needs owner |
| D8 | "Never clear-text password authentication" | A small probe-only PostgreSQL client that accepts SCRAM-SHA-256 only. See finding F5. | *(proposed)* — needs owner |
| D9 | MySQL | Out of scope: not in the Target table. `sqlx` already refuses `mysql_clear_password` by default. | spec |
| D10 | Dagster-side TLS | Out of scope: the Target table names the *test* only. D1 covers ingest runs. | spec |

## 2. What exists today (anchors, verified at `29ca545`)

The five Rust files below are identical on `origin/main` `cd8e3df`. Re-find
by symbol if a line has moved.

### Findings: spec against code

| # | Spec row / claim | Code today | Verdict |
| --- | --- | --- | --- |
| F1 | PUT ingest-spec keeps the stored password on a host change | `lakehouse-store/src/connectors.rs:1077` `set_ingest_spec` updates `adapter`, `dial` and three more columns and never touches `secret_ref`. `routes/connectors.rs:2708` `ingest_spec_put` checks cron, load modes, upload targets and SSRF only. `IngestSpecBody` (`:2559`) has no credential field. | Gap confirmed. Nothing built. |
| F2 | Dial override on PUT credential | `routes/connectors.rs:1429` `candidate_dial_info` replaces `candidate.dial` with the request's `dial` and keeps the stored ref of any slot not sent. `set_credential` (`:1568`) then probes it. Sending only `primary` plus a new `dial` sends the stored secondary to the new host. | Gap confirmed. Nothing built. |
| F3 | *Not in the spec* | `PATCH /api/connectors/{id}` accepts `host` (`routes/connectors.rs:297`, store `connectors.rs:1640`). `connector.host` is what these dial: `probe_postgres` for a row with no adapter (`connector_probe.rs:868`), the CDC delete for a legacy row (`routes/connectors.rs:2207`), `probe_s3` (`connector_probe.rs:1026`). | Third path, same attack for legacy PostgreSQL rows. Needs D4. |
| F4 | PostgreSQL test TLS is `Prefer` | `connector_probe.rs:909`. Also `connector_discover.rs:323` and `connector_deprovision.rs:258`, which the spec does not name. | Gap confirmed in three places, not one. |
| F5 | Never clear-text password authentication | `sqlx-postgres` 0.8.6 `connection/establish.rs:74` answers `AuthenticationCleartextPassword` with the password, unconditionally. It has no setting to refuse. | Gap confirmed. Cannot be fixed with a `sqlx` option. |
| F6 | `sslmode=require` at least | The compose PostgreSQL does not terminate TLS (`connector_probe.rs:904-908` says so). `require` makes the seeded connector's test fail on a default install. | Target conflicts with the default stack. Needs D7. |
| F7 | SQL Server test: "Encryption optional" | `tiberius` 0.13.0 `client/config.rs:242` defaults to `EncryptionLevel::Required` with the `rustls` feature this crate enables. Encryption is already required. The gap is `config.trust_cert()` (`connector_probe.rs:488`, `connector_discover.rs:359`): the certificate is not verified. | Spec's "Today" is inaccurate. Correct it by name (principle 5). |
| F8 | Assistant | `routes/ai/tools/ingest.rs:48` `set_ingest_spec` calls `ingest_spec_put` directly. | Inherits the fix. Schema must not gain a credential field. |
| F9 | Console | `src/features/connectors/connector-edit-page.tsx:187-229` sends the credential first (with the new `dial`), then the ingest spec, and allows a connection change with no credential typed. | Must change with the API. |

TLS alone does not close F1–F3. `require` without a CA still completes a
handshake with any server, and a SQL Server login carries the password
inside that tunnel. The re-point rule (tasks 1–4) is the fix; TLS (tasks
5–7) limits what a network attacker on the real path can do.

## 3. Target identity (D1)

One function, `Dial::target_identity`, in
`lakehouse-store/src/ingest_spec.rs`, next to `secret_map_auth_type`. It
returns the adapter name plus:

| Adapter | Fields compared |
| --- | --- |
| `sql`, `cdc` | `driver`, `host`, `port`, `database` |
| `files` | `protocol`, `endpoint`, `bucket` |
| `rest` | scheme, host and port of `baseUrl` |
| `mongodb` | `hosts`, `database` |
| `kafka` | `bootstrapServers` |
| `sftp` | `host`, `port`, `hostKeyFingerprint` |
| `sheets` | none (fixed Google hosts) |

Host names compare case-insensitively. A change of `user`, `sourceObjects`,
schedule, pagination or `sslMode` is not a re-point.

## 4. Tasks

Rust tasks 1–7 sit together; no migration is needed.

1. **`Dial::target_identity`.** As §3.
   *Check:* unit tests in `ingest_spec.rs`, one per adapter, each asserting
   that a changed compared field differs and a changed non-compared field
   does not.

2. **Store: re-point and credential swap in one transaction.** Add
   `connectors::repoint_ingest_spec(pool, id, spec, swaps)` beside
   `set_ingest_spec`: it locks the row, writes the spec and applies the
   `SecretRefSwap`s (reuse `swap_secret_refs`' guarded `UPDATE`s) or rolls
   back all of it. `set_ingest_spec` itself gains a guard: it returns
   `StoreError::Conflict` when the stored and new target identities differ,
   so no caller can re-point without the new function.
   *Check:* `lakehouse-store/tests/connector_ingest_spec.rs` — a host change
   through `set_ingest_spec` is `Conflict` and the row is unchanged; the
   same change through `repoint_ingest_spec` writes spec and refs together.

3. **`PUT /api/connectors/{id}/ingest-spec`.** `IngestSpecBody` gains an
   optional `credential { primary, secondary }` (reuse `CredentialSlotBody`).
   Order in the handler: validate, read the stored spec, compare identities.
   Unchanged and no credential: today's path. Changed and any slot missing
   (D3): `409` with fixed text, before any DNS lookup or dial. Changed with
   credentials: probe the candidate with `CandidateSecretResolver` (as
   `set_credential` does), refuse a failed probe with `422`, then stage the
   files and call `repoint_ingest_spec`, rolling the files back if it fails
   (reuse `replace_credentials`' stage/publish/rollback; extract the shared
   part instead of copying it). Audit `connector.repoint`: slots and
   `verified`, never a ref or value.
   *Check:* new `lakehouse-api/tests/connector_repoint.rs`. The regression
   test stands up a local listener as the "new" host, re-points the seeded
   `conn-pg-lakehouse` without credentials, asserts `409`, then calls
   `POST .../test` and asserts the listener received no connection. Further
   cases: host, port, database and adapter each `409`; a `rest` `baseUrl`
   change `409`; one slot of two `409`; an unchanged target saves as before.

4. **`PUT /api/connectors/{id}/credential` and `PATCH` `host`.**
   `candidate_dial_info`: when the request's `dial` changes the target
   identity, every slot must be in the request, otherwise `409` and no
   probe. `PATCH` (if D4 is signed): a `host` change on a row that dials
   from `host` (no adapter, or `files`) is refused with a pointer to the
   ingest-spec route, in `reject_non_patchable_fields`' style.
   *Check:* `connector_repoint.rs` — `primary` only plus a re-pointing
   `dial` is `409` and the listener sees nothing; both slots pass. `PATCH`
   `host` on a legacy row is refused.

5. **PostgreSQL TLS.** One helper in `connector_probe.rs` builds the
   options for all three dials (`connector_probe.rs:898`,
   `connector_discover.rs:317`, `connector_deprovision.rs:248`):
   `PgSslMode::Require`, or `VerifyFull` with `ssl_root_cert` when
   `CONNECTOR_TLS_CA_FILE` is set. Add the setting to `config.rs` and
   rewrite the three "compose does not terminate TLS" comments.
   *Check:* a unit test per mode on the helper; a probe against a plaintext
   listener fails with the fixed "TLS error" class and the listener never
   receives a startup packet carrying a password.

6. **PostgreSQL: no clear-text authentication (D8).** The test dial stops
   using `sqlx`. A probe-only client does TLS, startup, SCRAM-SHA-256 and
   `SELECT 1`, and returns a fixed "server asked for an authentication
   method this console refuses" for clear-text and MD5 requests. `hmac`,
   `sha2`, `base64` and `rand` are already workspace dependencies.
   *Check:* a fake server that answers `AuthenticationCleartextPassword`
   receives no `PasswordMessage`; the same for MD5; a real SCRAM server
   passes (the gate PostgreSQL).

7. **SQL Server TLS.** State `config.encryption(EncryptionLevel::Required)`
   explicitly in both dials. With `CONNECTOR_TLS_CA_FILE` set, use
   `trust_cert_ca` instead of `trust_cert()`. Rewrite the comment at
   `connector_probe.rs:483`.
   *Check:* unit test on the built `Config`; a probe against a server that
   offers no encryption fails with the "TLS error" class.

8. **Compose (D7).** An init container (`restart: "no"`, idempotent)
   creates a CA and server certificate into a volume; PostgreSQL starts
   with `ssl=on`; the API mounts the CA and sets `CONNECTOR_TLS_CA_FILE`.
   Document the setting in `.env.example`.
   *Check:* `docker compose up` from a clean project; the seeded
   connector's test passes over `verify-full` (rule 8).

9. **Console.** `IngestSpecInput` gains the optional `credential`.
   `connector-edit-page.tsx`: when the edit changes a compared field the
   credential inputs become required, and the credential travels in the
   ingest-spec request instead of a separate call. A `409` shows the
   server's message.
   *Check:* `connector-edit-page.test.tsx` — a host change with no
   credential cannot be submitted; with one, a single ingest-spec request
   carries it.

10. **Assistant (AI team reviews).** `set_ingest_spec`'s tool schema gets
    no credential field: the assistant must never carry a secret. Its
    description and the `prompt.rs:265` line say a re-point needs the
    console. A `409` reaches the model as the fixed message.
    *Check:* a tool test that a re-point returns the `409` text;
    `tests/fixtures/tool_schemas.json` regenerated only if the description
    changed.

11. **Docs.** Correct `specs/sec-14.md`'s SQL Server "Today" cell (F7) and
    add F3 if D4 is signed; `CHANGELOG`; `SECURITY.md` if it lists this.

## 5. PR slicing

| PR | Tasks | Why separate |
| --- | --- | --- |
| A | 1–4, 9, 10 | Closes the theft path. Ships first; needs no compose change. |
| B | 5–8, 11 | TLS. Blocked on D7 and D8. |

## 6. Out of scope

- `SEC-15` (internal addresses, redirects, address pinning) — its own plan;
  it touches the same dials, so build it after PR B, not beside it.
- MySQL and Dagster-side TLS (D9, D10).
- Re-keying credentials already exposed. If a deployment may have been
  attacked, its connector passwords are rotated by the operator.
- The default `lakehouse` password at `docker-compose.yml:297` (rule 5) —
  already red; needs its own backlog item.

## 7. Not verified

- No cargo, bun or compose command was run for this plan; it is a reading
  of the code.
- Whether any deployed row still has `adapter IS NULL` (F3's reach) was not
  checked against a database.
- `tiberius`' behaviour against a server that offers no encryption was read
  from its source, not observed.

## Handoff: PR A (tasks 1, 2, 3, 4, 9, 10)

Developer: Claude Sonnet 5.5. Branch `fix/sec-14-connector-repoint`, stacked
on `fix/sec-15-probe-internal-addresses`. Not pushed. PR B (tasks 5 to 8 and
11) is not started: `PgSslMode`, `MySqlSslMode` and `trust_cert` are
untouched.

### Commits (one per task, plan order except 10 before 9)

| Task | Commit | What |
| --- | --- | --- |
| 1 | `a0e527a` | `Dial::target_identity`, `is_repoint`, unit tests in `ingest_spec.rs` |
| 2 | `15f01de` | `set_ingest_spec` guard (`Conflict`), `repoint_ingest_spec`, shared `apply_swaps`, tests in `tests/connector_ingest_spec.rs` |
| 3 | `1cbd22b` | `PUT .../ingest-spec` with `credential`, `publish_credentials` extracted from `replace_credentials`, `tests/connector_repoint.rs`, `route_auth.rs` SSRF test given a credential |
| 4 | `b78ffa9` | `candidate_dial_info` 409, `PATCH` `host` refusal, unit and route tests |
| 10 | `69307cb` | tool description, prompt line, `tool_schemas.json` by hand, schema pin test |
| 9 | `1d709ac` | `target-identity.ts` + tests, `IngestSpecInput.credential`, edit page, edit-page tests |

### Decisions taken where the plan was silent

- **Target identity** is as section 3, plus: the adapter name is part of it;
  `mongodb` hosts and `kafka` bootstrap servers compare as sets (order,
  repeats and host case ignored); an absent `files.endpoint` differs from a
  named one; a URL's absent port is the scheme's default (`http` 80,
  `https` 443) and its userinfo is ignored; `sql`, `cdc`, `sftp`, `mongodb`
  and `kafka` have no absent port (`Dial::parse` requires one), so there is
  no default to apply. The same list is in the doc comment of
  `Dial::target_identity`.
- **First configuration** (`is_repoint`): stored `adapter` NULL and `dial`
  `{}`/null is not a re-point. Anything that cannot be compared fails
  closed: a stored `adapter` whose `dial` does not parse for it, or
  `adapter` NULL with a non-empty `dial`.
- **Which slots** (D3) are counted from `secret_field_names` for the NEW
  adapter and auth type (the credentials the dial actually reads): one field
  needs `primary`, two need `primary` and `secondary`, none (Kafka without
  authentication) need nothing and a credential sent for it is a 400.
- A `credential` sent with an **unchanged** target is accepted: probed and
  written with the spec in the same way, audited as
  `connector.credential_set`. Coverage is only required when the target
  changes.
- **`PATCH` `host`**: refused (409, fixed text) only for a connector with no
  adapter or a `files` adapter, the two whose probe reads `connector.host`
  (`connector_probe.rs`, `probe_by_kind` and `probe_s3`). For every other
  adapter `host` is a label and still works. A value equal to the stored one
  (ignoring case and padding) is accepted. The plan said "refused with a
  pointer, in `reject_non_patchable_fields`' style" (a 400); it is a 409
  with the same pointer, because D4 says it follows the same rule as D1.

### Where the plan differed from the code after SEC-15

- `check_dial_ssrf` ran right after `Dial::parse` in `ingest_spec_put`. It
  now runs after the stored spec is read and the identities are compared, so
  a re-point without credentials is refused before any DNS lookup.
- `ingest_spec_put` had no principal. It now takes
  `Option<Extension<Principal>>` (like `update`) for the audit event. The
  assistant's tool passes `None`: it never carries a credential, so it never
  reaches the part that needs one.
- `tests/route_auth.rs::ingest_spec_put_rejects_a_dial_whose_host_resolves_internal`
  moves `conn-pg-lakehouse` from `sql` to `rest`, a re-point, so it would now
  get the 409 instead of the address check's 400. Its body gained a
  throwaway bearer credential; the assertion (400) is unchanged. This is the
  only existing Rust assertion touched.
- The seeded `conn-pg-lakehouse` dial host is `postgres`, which does not
  resolve outside the compose network, so route tests that need a save to
  pass use their own rows dialling `127.0.0.1`.
- `tool_schemas.json` is normally rewritten by the ignored test
  `write_tool_schema_fixture`; that needs a test binary, which this machine
  must not build, so the one changed `description` string was edited by hand
  in the fixture and in `registry.rs` identically.

### Existing assertions changed

- `rust/crates/lakehouse-api/tests/route_auth.rs`: see above (credential added to the body).
- `src/features/connectors/connector-edit-page.test.tsx`:
  `keeps the ingested tables and schedule when the connection changes` used
  to change the host and expect a separate credential call first. It now
  changes the user (a save that keeps the target), asserts no credential call
  and no `credential` key, and the host change has its own new test. No other
  assertion was weakened.

### Commands run (and results)

- `cd rust && cargo fmt --check`: pass.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  (with `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-catalog-target`,
  `CARGO_BUILD_JOBS=2`; `df -h /` showed 65 GB free before each run): pass,
  no warnings.
- `bun run typecheck`: pass. `bun run lint`: 0 errors, 6 warnings, none in
  files this PR touches. `bun run test`: 883 pass, 1 skip, 0 fail
  (includes 15 new tests in `target-identity.test.ts` and 3 new in
  `connector-edit-page.test.tsx`).
- `python3 ops/lint/check_intra_package_imports.py` and
  `python3 ops/lint/check_bare_iceberg_count.py`: pass.

### Not verified

- **No Rust test was run.** `cargo test` builds test binaries, which this
  machine must not do. The new Rust tests (`ingest_spec.rs`
  `target_identity_tests`, `tests/connector_ingest_spec.rs`,
  `tests/connector_repoint.rs`, the unit tests in `routes/connectors.rs`,
  the schema pin in `registry.rs`) type-check under clippy but have never
  executed; the first run is CI's. The snapshot test
  `tool_schemas_snapshot_is_byte_identical` rests on the hand edit.
- `tests/connector_repoint.rs::a_re_point_with_credentials_is_probed_and_saved_together`
  dials the test Postgres container as a real new target and reads the
  managed credential back from a temporary `CONNECTOR_SECRETS_DIR`; it has
  not been seen to pass.
- The compose stack, the Dagster side and a browser were not run.
- For a `files` connector the candidate probe reads `connector.host`
  (`probe_s3`), not the new `dial`, so the new endpoint is not tested before
  the save; the credentials sent are not delivered to the old place by this
  (the request's credentials only), but the check is weaker than for `sql`
  and `rest`.
- A connector with no adapter and an operator-provisioned credential takes
  its first dial without credentials, by the boundary above. That is the
  same row class that `PATCH` `host` now refuses to move.
