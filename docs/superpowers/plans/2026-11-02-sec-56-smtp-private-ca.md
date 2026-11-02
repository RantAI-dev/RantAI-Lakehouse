# SEC-56: email through an internal mail relay with a private CA

Backlog `SEC-56` (Task, Administration & Security, P2). Found in `BI-19` owner QA on
2026-10-28 (QA-5, `docs/superpowers/plans/2026-10-18-bi-19-sharing.md:1367-1374`); the owner
made it a separate item. Base: `main` at `9c066426`. Every `file:line` below was read at that
commit. Planner: Claude Opus. Developer: a different agent (`AGENTS.md`). The developer does
not change a decision below; if the code disagrees, stop and report it.

## 0. What is wrong, with evidence

- One mail sender serves every email the product sends (alerts, digests, chart alerts,
  scheduled reports, comment mentions, pipeline and load alerts):
  `lakehouse_notify::EmailSender` (`rust/crates/lakehouse-notify/src/lib.rs:265`), built from
  `routes::alerts::smtp_config` (`rust/crates/lakehouse-api/src/routes/alerts.rs:641`) at
  `alerts.rs:1141`, `ai/tools/alerts.rs:147`, `authored_pipelines.rs:1185`,
  `dashboard_comments.rs:665`, `load_alerts.rs:308`, `:343`, `pipelines.rs:2677`, `:2875`,
  `report_runner.rs:1351`, `reports.rs:810`.
- `EmailSender::transmit` (`lib.rs:341-367`) uses `AsyncSmtpTransport::relay(host)` when
  `SMTP_SECURE` is in effect and `starttls_relay(host)` otherwise (`:342-346`). Both build
  `TlsParameters::new(host)`, which trusts only the built-in root store.
- `lettre` is built with `rustls-tls` (`rust/Cargo.toml:78`), which is `webpki-roots` +
  `rustls` + `ring` (`lettre-0.11.23/Cargo.toml:119-123`). With neither
  `rustls-native-certs` nor `rustls-platform-verifier`, `CertificateStore::Default` loads the
  Mozilla roots only (`lettre-0.11.23/src/transport/smtp/client/tls.rs:493-511`). So a relay
  whose certificate chains to a company CA fails the handshake, and every send ends
  `MailServerUnreachable` ("the mail server could not be reached", `lib.rs:72`), the same
  sentence as a wrong host. There is no setting to change that: `SmtpConfig` has six fields
  (`lib.rs:120-136`), none about trust.
- `lettre` already has what is needed, with no new feature: `TlsParameters::builder(domain)`
  (`tls.rs:603`), `TlsParametersBuilder::add_root_certificate` (`:248`), which adds to the base
  store rather than replacing it (`:497-526`, the loop at `:518`), `Certificate::from_pem`, which reads every
  certificate in a PEM bundle (`:676-698`), and `AsyncSmtpTransportBuilder::tls`
  (`async_transport.rs:440`) with `Tls::Required` (STARTTLS, no plaintext fallback) and
  `Tls::Wrapper` (implicit TLS) (`tls.rs:118`, `:131`).
- The repo already has the same pattern for connector dials: `CONNECTOR_TLS_CA_FILE`
  (`SEC-14` part B), read and checked at start by `parse_connector_tls_ca_file`
  (`rust/crates/lakehouse-api/src/config.rs:1333-1356`), with `ConfigError` variants
  `UnreadableConnectorTlsCaFile` / `EmptyConnectorTlsCaFile` (`config.rs:161-173`) and tests
  (`config.rs:2723-2790`).

## 1. Decisions

- **D1. One optional setting, `SMTP_CA_FILE`:** the path, inside the API container, of a PEM
  file with one or more CA certificates. Unset or empty: behaviour is exactly today's (the
  transport is built the same way, byte for byte). Compose passes it as
  `SMTP_CA_FILE: ${SMTP_CA_FILE:-}` (optional, no default).
- **D2. The CAs are added to the built-in roots, not put in their place**
  (`add_root_certificate` on `CertificateStore::Default`). A deployment that sets the file can
  still reach a public relay; the backlog asks for "a way to trust a private CA", not a pin.
  *(planner default)*
- **D3. Never weaker than today.** The setting applies to both modes: `SMTP_SECURE` in effect →
  `Tls::Wrapper`; otherwise → `Tls::Required` (STARTTLS required, no fallback). No code path
  uses `Tls::None`, `Tls::Opportunistic`, `dangerous_accept_invalid_certs` or
  `dangerous_accept_invalid_hostnames`; the hostname check stays on, so `SMTP_HOST` must match
  a name on the relay's certificate. A test pins this (T2, case 4).
- **D4. No plaintext mode, not even to loopback.** *(planner default; owner to confirm, since
  the backlog row offered it as an alternative)* In Compose the API runs in its own container,
  so "loopback" is the API container itself; a relay sidecar is on the Compose network, not
  on loopback. The condition could not be checked in the deployment the product ships, and a
  plaintext hop would carry `SMTP_PASS`. A relay without TLS is not supported; the operator
  docs say so honestly.
- **D5. Fail closed at start.** A set `SMTP_CA_FILE` that cannot be read, or that holds no PEM
  certificate, stops the API at start with an error naming the variable, the same rule and
  the same checks as `CONNECTOR_TLS_CA_FILE`, through **one shared helper** (rule 4): generalise
  `parse_connector_tls_ca_file` to take the variable's name, and keep a pair of `ConfigError`
  variants per variable so each message names its own setting.
- **D6. Read once, at start.** The file is parsed into certificates when the configuration is
  read and kept in memory. A replaced file takes effect at the next restart; the docs say so.
  *(planner default)* Reason: the start-up check is the fail-closed point (D5), and the send
  path then does no file I/O.
- **D7. A TLS failure has its own fixed reason.** *(planner default)* Add
  `DeliveryFailure::MailServerTlsFailed`: "a secure connection to the mail server could not be
  made", chosen when `lettre`'s error says `is_tls()` (`lettre-0.11.23/src/transport/smtp/error.rs:76`).
  It covers an untrusted certificate and a hostname mismatch, and it is the sentence that tells
  an operator to look at `SMTP_CA_FILE` and `SMTP_HOST`. The certificate's text and the
  server's reply never reach a response (`SRC-17` F1, principle 4). The report runner's closed
  mapping (`report_runner.rs:539-555`) gains its sentence.
- **D8. The mount.** *(planner default)* Compose mounts an operator directory read-only:
  `${SMTP_CA_DIR:-./ops/smtp-ca}:/run/certs/smtp:ro` on `lakehouse-api`, with an empty
  `ops/smtp-ca/.gitkeep` in the repo. The operator puts `ca.pem` there and sets
  `SMTP_CA_FILE=/run/certs/smtp/ca.pem`. The default is a path, not a credential (rule 5). No
  certificate or key is ever committed (gitleaks reads a committed key as a leak; tests mint
  theirs, as `rcgen` does for `SEC-14`/`SEC-41`).
- **D9. The certificates are not secret, but the type says what it holds.** The bundle lives
  in a `lakehouse_notify::SmtpCaBundle` with a hand-written `Debug` that prints the count only
  (lettre's `Certificate` has no `Debug`), so `SmtpConfig` keeps its `derive(Debug)`.

## 2. Anchors (verified at `9c066426`)

| What | Where |
| --- | --- |
| Sender | `rust/crates/lakehouse-notify/src/lib.rs`: `DeliveryFailure` `:39`, `message` `:61-75`, `SmtpConfig` `:120-136`, `EmailSender` `:265`, `send` `:294`, `send_with_attachments` `:317`, `transmit` `:341-367`; tests `mod tests` `:458`, `cfg()` `:627`, the pinned sentences `:775-812` |
| Struct literals of `SmtpConfig` | `routes/alerts.rs:641-650` (`smtp_config`), `routes/alerts.rs:1991`, `lakehouse-alerts/src/lib.rs:4634`, `lakehouse-notify/src/lib.rs:628` |
| Config | `rust/crates/lakehouse-api/src/config.rs`: `ConfigError` `:55`, connector CA variants `:161-173`, SMTP fields `:394-427`, `Debug` `:1044-1049`, SMTP parse `:1409-1446`, `parse_connector_tls_ca_file` `:1333-1356` (called `:1451-1453`), `connector_tls_ca()` `:1712`, its tests `:2723-2790` |
| Report failure mapping | `rust/crates/lakehouse-api/src/routes/report_runner.rs:539-555` |
| Compose | `docker-compose.yml:313-318` (SMTP env of `lakehouse-api`), `:758-760` (its volumes; the connector CA mount is the model) |
| Env template | `.env.example:276-282` (SMTP block), `:827-835` (connector CA wording to mirror) |
| Operator docs | `docs/OPERATIONS.md:1135` (the SMTP row of the dependency table) |
| lettre | `~/.cargo/registry/src/index.crates.io-*/lettre-0.11.23/src/transport/smtp/client/tls.rs`: `Tls` `:78`, `CertificateStore` `:161`, `TlsParametersBuilder` `:213`, `add_root_certificate` `:248`, `build_rustls` `:446-560`, `TlsParameters::builder` `:603`, `Certificate::from_pem` `:676`; `async_transport.rs`: `relay` `:135`, `starttls_relay` `:169`, `port` `:404`, `tls` `:440`; `error.rs` `is_tls` `:76` |
| Test tooling already in the lock | `testcontainers 0.27` (`lakehouse-test-support/Cargo.toml:18`, `lakehouse-api/Cargo.toml:230`), `rcgen 0.14` with `ring` only (`lakehouse-api/Cargo.toml:267`) |
| A comment to correct | `rust/crates/lakehouse-api/src/crypto_provider.rs:34-36` says `lettre` falls back to `aws-lc-rs`; with `rustls-tls` it is `ring` (`lettre-0.11.23/src/rustls_crypto.rs:6-14`, `Cargo.toml:119-123`) — rule 1 |

## 3. Tasks (one commit each, in order)

Per commit: `cargo fmt --check`, `cargo clippy -p <crate> --all-targets -- -D warnings`, the
scoped tests named. All cargo through `/home/shiro/rantai/bin/cargo-locked`. Push after every
commit. Cite `SEC-56 D<n>` at each site and in the commit body.

**T1. The sender trusts a supplied bundle (`lakehouse-notify`).** D2, D3, D7, D9.
- `pub struct SmtpCaBundle` (wraps `lettre::transport::smtp::client::Certificate` and its count):
  `pub fn from_pem(pem: &[u8]) -> Result<Self, CaBundleError>` (`Empty` for zero certificates,
  `Invalid` for a PEM error), `pub fn len(&self)`, manual `Debug`.
- `SmtpConfig` gains `pub ca_bundle: Option<SmtpCaBundle>`; update the four struct literals
  (section 2) with `None`.
- `transmit`: when `ca_bundle` is `None`, the code path is unchanged. When it is set:
  `TlsParameters::builder(host.to_owned()).add_root_certificate(..).build()` (a build error is
  `MailServerTlsFailed`), then `relay(host)` / `starttls_relay(host)` as today followed by
  `.tls(Tls::Wrapper(params))` / `.tls(Tls::Required(params))`. No `builder_dangerous`, no
  `dangerous_*`, no `Opportunistic` (D3).
- `DeliveryFailure::MailServerTlsFailed` and its sentence; classify `err.is_tls()` before the
  transient/permanent branch; add it to the pinned-sentence test.
- Unit tests (no network): `from_pem` of an empty file, of text that is not PEM, of one and of
  two certificates (minted with `rcgen` in the test, never committed); `Debug` of a config with a
  bundle prints a count and no PEM.
- Dev-dependencies of `lakehouse-notify`: `rcgen` with exactly the features `lakehouse-api`
  pins. If PEM output needs `rcgen`'s `pem` feature, do **not** turn it on (it would unify into
  the API's tests); write the PEM armour in the test from the DER, as
  `lakehouse-api/src/tls_proxy.rs`'s `pem_certificate` does.
- *Accept:* `cargo test -p lakehouse-notify` and `cargo clippy -p lakehouse-notify --all-targets
  -- -D warnings`; then `cargo check -p lakehouse-alerts -p lakehouse-api --tests` (the struct
  literals compile).

**T2. Proof with a real relay in a test container (`lakehouse-notify/tests/smtp_private_ca.rs`).**
- A Mailpit container (`axllent/mailpit`, **pinned to an exact tag**; read that tag's docs and
  quote, in the handoff, the variables used for the SMTP certificate, the key, "require
  STARTTLS" and "require TLS"). The test mints, at run time, a CA and a server certificate
  signed by it with SANs for `localhost` **and** the host `testcontainers` reports, copies them
  into the container (`with_copy_to`), and reads delivered mail back through Mailpit's HTTP API
  with `reqwest` (already a dependency) and a `timeout`.
- Cases, each a sentence-named test:
  1. STARTTLS required, no bundle → `MailServerTlsFailed`, and Mailpit holds no message;
  2. STARTTLS required, the CA as `ca_bundle` → `ok`, and Mailpit holds exactly that subject;
  3. a bundle with a **different** CA → `MailServerTlsFailed`, nothing delivered;
  4. the right CA but `SMTP_HOST` set to a name that is not on the certificate (for example the
     IP when the SAN is only the name, or the reverse) → `MailServerTlsFailed` (D3: the hostname
     check stays on);
  5. implicit TLS (`secure: true`, Mailpit requiring TLS) with the CA → `ok`;
  6. a bundle of two certificates where the relay's CA is the second → `ok`.
- The containers start the way `lakehouse-test-support` starts Postgres (same `testcontainers`
  version and arrangement), so the test runs wherever the workspace tests already run. If
  Mailpit at the pinned tag cannot require STARTTLS with a supplied certificate, **stop and
  report**; do not swap in a plaintext relay.
- Dev-dependencies: `testcontainers` with the workspace's version and features, `tokio`.
  `Cargo.lock` must gain no new crate **version** (only new dependency edges); prove it with
  `git diff rust/Cargo.lock` and `cargo tree -p lakehouse-notify -e dev --locked` in the handoff.
  If a new version appears, stop and report.
- *Accept:* `cargo test -p lakehouse-notify --test smtp_private_ca` (six passed, quoted), run
  three times in a row green; `cargo test -p lakehouse-notify --locked`.

**T3. The setting (`lakehouse-api` config, `smtp_config`, report mapping, comment fix).** D1, D5, D6, D7.
- Generalise `parse_connector_tls_ca_file` into one helper that reads a CA file for a named
  variable; `CONNECTOR_TLS_CA_FILE` keeps its exact behaviour and messages (its tests pass
  unchanged). `SMTP_CA_FILE` uses the helper, then `SmtpCaBundle::from_pem`;
  `ConfigError::UnreadableSmtpCaFile(PathBuf)` / `EmptySmtpCaFile(PathBuf)` with messages that
  name `SMTP_CA_FILE`.
- `Config.smtp_ca: Option<SmtpCaBundle>` (and the path for `Debug`); `smtp_config` passes it.
- `report_runner::email_failure_reason`: "The email was not sent: a secure connection to the
  mail server could not be made." for the new reason.
- `crypto_provider.rs:34-36`: say `ring`, with the `lettre` file and line (rule 1).
- *Accept:* `cargo test -p lakehouse-api config` (new: unset and empty → `None`; a readable
  bundle → `Some` with its count; a missing path and a directory → `UnreadableSmtpCaFile`; an
  empty file and non-PEM text → `EmptySmtpCaFile`; each message contains `SMTP_CA_FILE`; the
  connector tests unchanged), `cargo test -p lakehouse-api report_runner`, clippy for the crate.

**T4. Compose, `.env.example`, operator docs.** D1, D4, D6, D8.
- `docker-compose.yml`: `SMTP_CA_FILE: ${SMTP_CA_FILE:-}` beside the SMTP block (`:313-318`)
  with a why-comment (`SEC-56`, D2/D3/D5); the read-only mount of D8 beside the connector CA
  mount (`:760`). `ops/smtp-ca/.gitkeep`.
- `.env.example`: `SMTP_CA_FILE=` with a comment in the style of `:827-835`: what it is, that it
  adds to the built-in roots, that it is the path inside the container
  (`/run/certs/smtp/ca.pem` with the default mount), that a bad file stops the API, that a change
  needs a restart, and that a relay without TLS is not supported (D4).
- `docs/OPERATIONS.md`: a short "Email through an internal relay" section (the CA bundle, the
  name on the certificate must match `SMTP_HOST`, port 587 STARTTLS or 465 / `SMTP_SECURE=true`,
  the new failure sentence and what to check) and the dependency table row at `:1135`.
- *Accept (rule 8, a real `up`, not `config`):* from a clean project, `docker compose up -d
  lakehouse-api` (and what it depends on) three times: `SMTP_CA_FILE` unset → healthy;
  set to a valid bundle placed in `ops/smtp-ca/` → healthy; set to a missing path → the API
  exits and its log names `SMTP_CA_FILE`. Quote the commands, the health states and the log line.
  Plus `docker compose --profile '*' config --quiet`.

## 4. Out of scope

- Plaintext SMTP, opportunistic STARTTLS, "accept invalid certificates" (D3, D4).
- Client certificates (mutual TLS) to the relay (`identify_with`); a later item if asked.
- Trusting the operating system's store (`rustls-native-certs`): a new `lettre` feature and a
  different trust story; the bundle covers the case.
- **Found while planning, separate:** `SmtpConfig.pass` is a `String` in a `derive(Debug)`
  struct (`lakehouse-notify/src/lib.rs:120-133`), so a `{:?}` of it prints `SMTP_PASS`. The
  API's own `Config` redacts it (`config.rs:1048`), but the notify type does not use
  `SecretValue` (`AGENTS.md`, Rust non-negotiables). Proposed as `SEC-57`.
- The installer (`lakehousectl`, REL-2) setting `SMTP_CA_DIR`; the Compose default covers a
  repo checkout.
- `BI-38`'s "mention email not seen end to end": this item makes it possible; the owner's QA
  checks it.

## 5. Verification before handoff

```bash
cd rust && cargo fmt --check && cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p lakehouse-notify --locked && cargo test -p lakehouse-api && cargo test -p lakehouse-alerts
docker compose --profile '*' config --quiet   # plus T4's three real `up`s
git status                                    # no certificate, key, or ops/smtp-ca/*.pem staged
```

The PR touches dependencies (`lakehouse-notify` dev-dependencies, `Cargo.lock`): it must not
merge with a dependency check red (`AGENTS.md` step 6), and the reviewer re-runs
`cargo test -p lakehouse-notify --locked` on a clean tree. Under the owner's rule of
2026-10-30, CI's workspace run is the final one; anything not run locally is *not verified*,
with the reason.

## 6. Handoff (developer appends below)

### Developer report (developer, 2026-11-02)

One mismatch between the plan and the code, in D7's MECHANISM (the decision
itself is implemented as written). The plan says the new reason is "chosen
when `lettre`'s error says `is_tls()`". In `lettre` 0.11.23 that is true
only for a TLS *configuration* that could not be built:

- `lettre-0.11.23/src/transport/smtp/error.rs:76` — `is_tls()` is
  `matches!(self.inner.kind, Kind::Tls)`;
- `Kind::Tls` is produced only by `error::tls(...)`
  (`lettre-0.11.23/src/transport/smtp/error.rs:202`), and the only callers
  are in `transport/smtp/client/tls.rs` — every one of them a CONFIG build
  step (`build_rustls`, `from_der`, `from_pem`, ...);
- a handshake that FAILS — an untrusted certificate, or a name that is not
  on the certificate — is reported by `lettre` as a CONNECTION error:
  `lettre-0.11.23/src/transport/smtp/client/async_net.rs:362`
  (`map_err(error::connection)`), the same for the STARTTLS upgrade at
  `async_net.rs:296`. `tokio-rustls` puts the `rustls` error inside an
  `io::Error` (`tokio-rustls-0.26.6/src/common/mod.rs:115`).

So `is_tls()` alone would classify every case the plan's T2 cases 1, 3 and
4 name as `MailServerUnreachable`, and D7's own words ("It covers an
untrusted certificate and a hostname mismatch") would not hold.

What the code does instead: `lakehouse_notify::smtp_tls::is_failure` checks
`is_tls()` AND walks the error's source chain for a `rustls::Error` (the
`io::Error`'s `get_ref()`), so BOTH shapes are "a secure connection to the
mail server could not be made". The decision D7 states is what the code
does; only the detection differs, and the reason is at the site
(`rust/crates/lakehouse-notify/src/smtp_tls.rs`).

Two smaller things the plan does not name, both forced by the code:

1. `SmtpCaBundle` holds each certificate's DER rather than the transport's
   own `lettre::transport::smtp::client::Certificate`. That type has no
   `PartialEq` (and `Config` derives `PartialEq`), and its `Debug` is
   useless for D9; the DER bytes are what the transport is given either
   way. D9 is implemented as stated (the bundle's `Debug` prints its count
   and nothing else).
2. `lakehouse-notify` gained two direct dependencies: `rustls-pki-types`
   (the PEM parser `lettre` itself uses, so a bundle of several
   certificates is counted and validated one at a time) and `rustls` (the
   error type the TLS failure is recognised by). Both are already in
   `Cargo.lock`; the lock gained no new crate VERSION, only dependency
   edges.

Also noted for the record: the plan's D9 says "lettre's `Certificate` has
no `Debug`" — it has one
(`lettre-0.11.23/src/transport/smtp/client/tls.rs:701`), but it prints
`Certificate { }`, which is why the bundle's own `Debug` prints the count
instead.

Two things in the tests and the start-up path that the plan leaves open,
written down so a reviewer does not read them as scope changes:

3. T2 says "a `Mailpit` container", but its case 5 needs implicit TLS while
   the other cases need `STARTTLS`, and `Mailpit`'s
   `MP_SMTP_REQUIRE_STARTTLS` and `MP_SMTP_REQUIRE_TLS` cannot share a
   port. Each case therefore starts its own `axllent/mailpit:v1.33.0`
   container, and `testcontainers` removes it when the test's `Relay` is
   dropped (checked after the run: no relay container left).
4. A `SMTP_CA_FILE` whose PEM block does not decode is reported as
   `EmptySmtpCaFile` ("found none in ..."), not as a separate "invalid"
   variant. That wording is inherited, not new: `CONNECTOR_TLS_CA_FILE` has
   always reported a broken block the same way, and D5 asks for the same
   checks and the same messages. `SmtpCaBundle::from_pem` itself does tell
   the two apart (`CaBundleError::Empty` / `Invalid`) for a caller that
   wants it; the start-up path keeps the connector's wording on purpose.

### Handoff (developer, 2026-11-02)

The plan has no task after T4, so this handoff closes T1-T4.

**Commits** (branch `fix/sec-56-smtp-private-ca`, from `main` at `9c066426`,
merged with `main` at `bd76a56f`; nothing force-pushed, no PR opened):

| Commit | Task | What |
| --- | --- | --- |
| `a9379ee3` | T1 | `SmtpCaBundle`, `DeliveryFailure::MailServerTlsFailed`, the transport's TLS parameters |
| `cf8d8faf` | T2 | `tests/smtp_private_ca.rs`: a real `Mailpit` behind a private CA |
| `57802e27` | T3 | `SMTP_CA_FILE` in `Config`, the report runner's sentence, the `ring` comment |
| `59b64463` | T4 | Compose, `.env.example`, `docs/OPERATIONS.md`, `ops/smtp-ca/` |

**Run on the final commit (`59b64463`), all cargo through
`/home/shiro/rantai/bin/cargo-locked`:**

```bash
cd rust
cargo fmt --check
# exit 0. `-j 2` is not accepted by `cargo fmt` (cargo-fmt has no --jobs):
# it ran without it; the clippy and test runs below used `-j 2`.
cargo clippy -j 2 -p lakehouse-notify -p lakehouse-api --all-targets -- -D warnings
# exit 0, "Finished `dev` profile ... target(s) in 1m 31s"
cargo test -j 2 -p lakehouse-notify --locked
# 32 passed (unit, 0.01 s) + 6 passed (`tests/smtp_private_ca.rs`, 7.58 s)
#   + 0 doc-tests; exit 0
cargo test -j 2 -p lakehouse-api --lib config::
# 107 passed, 0 failed, 2843 filtered out; exit 0
cargo test -j 2 -p lakehouse-api --lib report_runner
# 16 passed, 0 failed, 2934 filtered out; exit 0
docker compose --profile '*' config --quiet
# exit 0
```

The six container tests were also run three times in a row in T2's
session, on `cf8d8faf` (that file is unchanged since): 6 passed in 7.4 s,
31.5 s and 26.7 s. Today's re-run is the 7.58 s above. `docker ps -a`
afterwards shows no `mailpit` container left behind.

**T4's three real `up`s (rule 8), run in T4's session from the clean project
`sec56` on the tree committed as `59b64463`; not re-run today, because
nothing in Compose, `.env.example`, `docs/OPERATIONS.md` or `ops/` has
changed since, and today's only change is this plan file.**

1. `SMTP_CA_FILE` unset — the API is healthy:
   `lakehouse-api running Up 26 seconds (healthy)`, and `GET /health` answers
   `{"status":"ok","database":"ok","version":"0.1.0"}`.
2. `SMTP_CA_FILE=/run/certs/smtp/ca.pem`, a bundle in `ops/smtp-ca/` — the
   API is healthy: `lakehouse-api running Up 15 seconds (healthy)`, the
   container's env carries `SMTP_CA_FILE=/run/certs/smtp/ca.pem`, and the
   file is there (`/run/certs/smtp/ca.pem` →
   `subject=CN=RantAI SEC-56 local relay CA`).
3. `SMTP_CA_FILE=/run/certs/smtp/missing.pem` — the API EXITS and its log
   names the setting:
   `lakehouse-api restarting Restarting (1) 3 seconds ago`, and
   `Error: failed to resolve configuration from environment` /
   `Caused by: SMTP_CA_FILE must name a readable file, cannot read
   "/run/certs/smtp/missing.pem"`.

Then `docker compose -p sec56 down -v` (project and volumes gone) and the
test certificate was deleted.

**Dependency check (`AGENTS.md` step 6; this PR touches dependencies).**
`git diff bd76a56f..HEAD -- rust/Cargo.lock` is ONE hunk, 5 added lines, all
of them dependency edges in `lakehouse-notify`'s own entry (`base64
0.23.1`, `rcgen`, `rustls`, `rustls-pki-types`, `testcontainers`). No
package block and no crate version was added. `cargo tree -p
lakehouse-notify -e dev --locked` names base64 0.23.1, rcgen 0.14.10,
testcontainers 0.27.3, tokio 1.54.1 and wiremock 0.6.5 — every one already
in the lock at the base commit.

**T2's relay tag and the variables it was read for (T2's accept).** The
image is `axllent/mailpit`, pinned to the exact tag `v1.33.0`. From that
tag's documentation, <https://mailpit.axllent.org/docs/configuration/smtp/>:

- `MP_SMTP_TLS_CERT` — "the certificate" the SMTP server serves (the test
  copies the minted server certificate in and points this at it);
- `MP_SMTP_TLS_KEY` — the matching private key (minted in the test too);
- `MP_SMTP_REQUIRE_STARTTLS=true` — "require STARTTLS": a plaintext session
  is refused and the client must upgrade;
- `MP_SMTP_REQUIRE_TLS=true` — "require SSL/TLS": the session is implicit TLS
  from the first byte (and, as report item 3 says, a separate container
  from the STARTTLS mode, since the two cannot share a port).

**Not verified, with the reason.**

- The full workspace block of `AGENTS.md` and plan §5 — `cargo clippy
  --workspace --all-targets --all-features`, `cargo test --workspace`,
  `cargo test -p lakehouse-alerts` — was not run locally: "Keep build time
  down" scopes a commit to its crates, and plan §5 leaves the workspace run
  to CI under the owner's rule of 2026-10-30. `cargo check -p
  lakehouse-alerts -p lakehouse-api --tests` was run in T1's session (the
  struct literals compile); today's `clippy -p lakehouse-api --all-targets`
  compiled the API's tests again, but not `lakehouse-alerts`' tests.
- `docker compose up` was not re-run today; only `config --quiet` (above).
  The real `up`s are T4's, quoted above.
- No TypeScript, Python, Dagster or ops-lint command was run: this PR
  touches none of those languages (its only non-Rust files are Compose,
  `.env.example`, markdown and `ops/smtp-ca/.gitkeep`), and T4's session ran
  `ops/lint/check_compose_init_readiness.py` and
  `ops/lint/check_image_digests.py` (both exit 0).
- The owner's acceptance checklist (step 7) and the D4 confirmation the
  planner flagged are the owner's and the planner's, not the developer's.

## 7. Review (planner appends below)


**Owner to confirm (planner, 2026-11-02):** D4. Planner default: no plaintext mode, not even to loopback. The `SMTP_PASS` Debug leak is backlog `SEC-59`.
