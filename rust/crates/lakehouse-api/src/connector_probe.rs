//! A REAL connectivity probe for `POST /api/connectors/{id}/test`, for the
//! connector types this build actually knows how to dial.
//!
//! # What this module supports, and why only this much
//!
//! Five connector shapes can be genuinely dialed today:
//!
//! - **`PostgreSQL`** (`sqlx`, already a `lakehouse-api` dependency for
//!   `lakehouse-store`) — opens a real connection and runs `SELECT 1`.
//! - **S3-compatible object storage** (`object_store`, already a workspace
//!   dependency via `lakehouse-iceberg`) — does an authenticated
//!   `list_with_delimiter` (a cheap, bounded listing call; no data read or
//!   written).
//! - **`MySQL`**/**`MariaDB`** (`sqlx`'s `mysql` feature, WS3 item 12) —
//!   same shape as `PostgreSQL`: connect, run `SELECT 1`.
//! - **Microsoft SQL Server** (`tiberius`, a new dependency, WS3 item 12)
//!   — connect, run `SELECT 1` via `simple_query`.
//! - **A `rest` adapter's REST/vendor API** (`reqwest`, already a
//!   `lakehouse-api` dependency) — one bounded `GET` against the dial's
//!   `baseUrl`, classified on status code only.
//!
//! The last two of those six are dispatched by `adapter`
//! (`sql`/`cdc`/`files`/`rest`/`sheets`, `0033_connector_ingest_spec.sql`),
//! not by the free-form `kind` label the first two still use — see
//! [`probe`]'s doc comment for the `adapter`-first-then-legacy-`kind`
//! dispatch rule.
//!
//! A `sheets` adapter is registered (`connector_type`, `0035`) but ships
//! **`supported: false` unconditionally** — see [`probe_sheets`]'s doc
//! comment for why (no verified Google service-account token exchange
//! exists in this workspace, WS3 plan review Z10 open question 4).
//!
//! Every other seeded/registered connector `type` (Kafka, MQTT, `MongoDB`,
//! Oracle, SAP/ERP, SFTP, ...) has **no dial implementation in this
//! build** — see [`probe_by_kind`]'s `_ =>` arm. Those report
//! [`Outcome::unsupported`], never a fabricated latency or success. Adding
//! a new supported type means adding both a real client dependency and a
//! new arm here — never widening the `_` arm to claim more than this
//! build can back up.
//!
//! # Credential handling
//!
//! [`probe`] receives a [`lakehouse_store::connectors::ConnectorDialInfo`]
//! (see that type's doc comment: no `Debug` impl at all) and a
//! [`DynSecretResolver`] to turn its `secret_ref`(s) into actual
//! [`SecretValue`]s (ADR 0002). **The resolver `probe` is handed must
//! already be scoped to a fixed allowlist** — see
//! `lakehouse_core::secret::AllowlistedSecretResolver` and
//! `crate::state::AppState::connector_secret_resolver`'s doc comment — NOT
//! the general-purpose `EnvSecretResolver` the rest of this process uses.
//! `probe` dials a `host` the SAME caller who supplied `secret_ref` also
//! controls (via `POST /api/connectors`), so an unrestricted resolver here
//! would let a `connector:manage` principal name any process secret
//! (`env:DATABASE_URL`, `env:CH_PASSWORD`, ...) and exfiltrate it to
//! infrastructure they own. The resolved value is used ONLY to build a
//! transient client (a `sqlx` connect string, an `object_store` builder
//! call) — it is never logged, never included in an [`Outcome::message`],
//! and never returned to the caller.
//!
//! # Error messages never echo upstream data
//!
//! A connection failure is reported as one of a small, fixed set of
//! generic failure classes (`classify_sqlx_error`/`classify_object_store_error`)
//! — "connection refused", "timed out", "authentication failed", "TLS
//! error", ... — never the underlying error's raw `Display` text. That
//! distinction matters specifically for `object_store`: a non-2xx HTTP
//! response from the dialed host is wrapped with the response body
//! included verbatim in its `Display` impl, and `probe_s3`'s `host`/`GET`
//! target is caller-controlled (see above), so echoing that text back
//! would turn `/test` into a working blind SSRF response-reader. Same
//! reasoning for `probe_postgres`.
//!
//! # SSRF: private/internal ranges are blocked before dialling
//!
//! [`probe_postgres`] and [`probe_s3`] resolve `host` via DNS
//! ([`resolve_checked`]) and refuse to dial it if ANY resolved address
//! falls in a private/internal range (RFC1918, loopback, link-local —
//! which covers cloud metadata endpoints at `169.254.169.254` — or IPv6
//! unique-local) — see [`is_blocked_ip`]. Checking is done against the
//! resolved address, not the literal `host` string: a hostname that
//! resolves to `10.0.0.5` is blocked exactly the same as a literal
//! `10.0.0.5`, so the check cannot be bypassed by pointing DNS at an
//! attacker-controlled name that merely LOOKS external. This is
//! `allow_internal_hosts: bool`-gated (default: blocked — see
//! `crate::config::Config::connector_probe_allow_internal_hosts`) because
//! this deployment's own seeded connectors
//! (`rust/migrations/0022_prune_connector_seed.sql`) legitimately point at
//! `postgres:5432` and `http://rustfs:9000`, both internal compose-network
//! names; a demo/compose deployment opts out of the block explicitly
//! rather than the block being off by default everywhere.
//!
//! # Timeouts
//!
//! Every dial is bounded by [`DIAL_TIMEOUT`] via [`tokio::time::timeout`]
//! — a hanging operator-configured host can delay a `/test` request by at
//! most that long, never indefinitely, and never ties up the request task
//! beyond it. This sits well inside `routes::DEFAULT_REQUEST_TIMEOUT`'s
//! 60s outer bound. No retries: one attempt, one measured result.

use std::net::IpAddr;
use std::time::{Duration, Instant};

use lakehouse_core::secret::DynSecretResolver;
use lakehouse_store::connectors::ConnectorDialInfo;
use lakehouse_store::ingest_spec::{CdcDial, Dial, RestAuth, RestDial, SqlDial, SqlDriver};
use object_store::ObjectStore;
use object_store::aws::AmazonS3Builder;
use sqlx::Connection;
use sqlx::mysql::{MySqlConnectOptions, MySqlConnection, MySqlSslMode};
use sqlx::postgres::{PgConnectOptions, PgConnection, PgSslMode};
use tokio_util::compat::TokioAsyncWriteCompatExt;

/// Bound on a single dial attempt (connect + one cheap operation). Chosen
/// to be a "few seconds" per the task brief — long enough that a healthy
/// LAN-local compose service never times out under normal load, short
/// enough that a hung/firewalled host resolves the request quickly.
///
/// `pub(crate)`: `connector_discover` reuses the SAME bound for its own
/// dial-then-list-schema attempt (WS3 item 14) rather than inventing a
/// second timeout constant for what is, mechanically, the same kind of
/// bounded network operation this module already disciplines.
pub(crate) const DIAL_TIMEOUT: Duration = Duration::from_secs(5);

/// The result of attempting (or declining to attempt) a connectivity
/// probe. Never carries a fabricated `latency_ms` — see the field doc
/// comment.
pub struct Outcome {
    /// Whether the probe succeeded. Always `false` when `supported` is
    /// `false`.
    pub ok: bool,
    /// Whether this build knows how to dial this connector's type.
    pub supported: bool,
    /// Real measured elapsed time, or `None` when `supported` is `false`
    /// (no attempt was made).
    pub latency_ms: Option<i64>,
    /// Human-readable result message.
    pub message: String,
}

impl Outcome {
    fn unsupported(kind: &str) -> Self {
        Self {
            ok: false,
            supported: false,
            latency_ms: None,
            message: format!(
                "This build cannot test a {kind:?} connector: no live-dial implementation \
                 exists for this connector type yet. Supported today: PostgreSQL, \
                 S3-compatible object storage."
            ),
        }
    }

    fn success(elapsed: Duration, message: impl Into<String>) -> Self {
        Self {
            ok: true,
            supported: true,
            latency_ms: Some(elapsed_millis(elapsed)),
            message: message.into(),
        }
    }

    fn failure(elapsed: Duration, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            supported: true,
            latency_ms: Some(elapsed_millis(elapsed)),
            message: message.into(),
        }
    }

    /// A supported type that could not even be attempted — a
    /// misconfigured `host`/`secret_ref` shape, or the referenced
    /// credential not resolving. Still `supported: true` (this build DOES
    /// know how to dial this type) but no dial occurred, so no latency
    /// exists to report.
    fn misconfigured(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            supported: true,
            latency_ms: None,
            message: message.into(),
        }
    }
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "a dial bounded by DIAL_TIMEOUT (5s) never approaches i64::MAX milliseconds"
)]
fn elapsed_millis(elapsed: Duration) -> i64 {
    elapsed.as_millis() as i64
}

/// Attempt a real connectivity probe for `info`, resolving its
/// `secret_ref`(s) via `resolver` (see the module doc comment: MUST already
/// be scoped to a fixed allowlist, never the general-purpose resolver).
/// `allow_internal_hosts` gates the SSRF blocklist — see the module doc
/// comment and [`Config::connector_probe_allow_internal_hosts`]. Never
/// panics on a malformed `host` or an unresolvable `secret_ref` — both
/// become [`Outcome::misconfigured`], never a crash.
///
/// Dispatches primarily on `info.adapter` (`sql | cdc | files | rest |
/// sheets`, `0033_connector_ingest_spec.sql`) rather than the free-form
/// `kind` label, falling back to the original `kind`-based dispatch only
/// for a pre-WS3 row with `adapter IS NULL` — mirrors the same
/// `adapter`-first-then-legacy-`kind` pattern the connector-deletion
/// deprovision step already uses (WS3 plan review X4).
///
/// [`Config::connector_probe_allow_internal_hosts`]: crate::config::Config::connector_probe_allow_internal_hosts
pub async fn probe(
    info: &ConnectorDialInfo,
    resolver: &dyn DynSecretResolver,
    allow_internal_hosts: bool,
) -> Outcome {
    match info.adapter.as_deref() {
        Some(adapter @ ("sql" | "cdc")) => {
            probe_dial(adapter, info, resolver, allow_internal_hosts).await
        }
        Some("rest") => probe_rest_info(info, resolver, allow_internal_hosts).await,
        Some("sheets") => probe_sheets(),
        // A `files` adapter's `dial` names an object-storage protocol (S3
        // today, per `lakehouse_store::ingest_spec::FilesProtocol`) — the
        // SAME thing the pre-WS3 `kind`-string dispatch below already
        // dials via `probe_s3`, which still reads `info.host`'s legacy
        // `<endpoint>|<bucket>` shape (`0022_prune_connector_seed.sql`'s
        // `conn-s3-warehouse` row keeps that shape after
        // `0034_seed_connector_ingest_spec.sql` sets `adapter = 'files'`
        // on it) — no second S3 client is added here.
        Some("files") => probe_s3(info, resolver, allow_internal_hosts).await,
        _ => probe_by_kind(info, resolver, allow_internal_hosts).await,
    }
}

/// The original `kind`-string dispatch (pre-WS3), kept as the fallback for
/// a connector row with `adapter IS NULL` — see [`probe`]'s doc comment.
async fn probe_by_kind(
    info: &ConnectorDialInfo,
    resolver: &dyn DynSecretResolver,
    allow_internal_hosts: bool,
) -> Outcome {
    let kind = info.kind.to_lowercase();
    if kind.contains("postgres") {
        probe_postgres(info, resolver, allow_internal_hosts).await
    } else if kind.contains("object storage") || kind.contains("s3") {
        probe_s3(info, resolver, allow_internal_hosts).await
    } else {
        Outcome::unsupported(&info.kind)
    }
}

/// The common connection fields [`SqlDial`] and [`CdcDial`] share — both
/// are "a [`SqlDriver`] plus `host`/`port`/`database`/`user`" — normalized
/// into one shape so [`probe_dial`] can dispatch by driver without caring
/// which of the two adapters (`sql` or `cdc`) produced it.
///
/// `pub(crate)` (fields included): `connector_discover` builds the SAME
/// shape from the SAME `Dial::Sql`/`Dial::Cdc` variants to decide which
/// driver-specific discovery connection to open (WS3 item 14) — reusing
/// this rather than a second private copy of the `From<&SqlDial>`/
/// `From<&CdcDial>` conversions below (AGENTS.md: grep for the existing
/// helper before writing one).
pub(crate) struct DialTarget<'a> {
    pub(crate) driver: SqlDriver,
    pub(crate) host: &'a str,
    pub(crate) port: u16,
    pub(crate) database: &'a str,
    pub(crate) user: &'a str,
}

impl<'a> From<&'a SqlDial> for DialTarget<'a> {
    fn from(dial: &'a SqlDial) -> Self {
        Self {
            driver: dial.driver,
            host: &dial.host,
            port: dial.port,
            database: &dial.database,
            user: &dial.user,
        }
    }
}

impl<'a> From<&'a CdcDial> for DialTarget<'a> {
    fn from(dial: &'a CdcDial) -> Self {
        Self {
            driver: dial.driver,
            host: &dial.host,
            port: dial.port,
            database: &dial.database,
            user: &dial.user,
        }
    }
}

/// Route a `sql`/`cdc` adapter's `dial` to the right driver-specific
/// probe.
///
/// `conn-pg-lakehouse` (`0034_seed_connector_ingest_spec.sql`) seeds
/// `adapter = 'sql'`, `dial.driver = "postgres"` on the SAME connector row
/// [`probe_postgres`]'s legacy `host`-string parsing (`parse_postgres_host`)
/// already dials successfully — after this task, that row's `adapter` is
/// no longer `NULL`, so [`probe`]'s dispatch reaches this function for it,
/// not [`probe_by_kind`]. Rather than add a second, untested
/// Postgres-from-`dial` code path, `SqlDriver::Postgres` here simply
/// re-enters [`probe_postgres`] via `info.host`, which is unchanged and
/// still valid. Only `Mysql`/`Mssql` get a genuinely new probe (WS3 item
/// 13).
async fn probe_dial(
    adapter: &str,
    info: &ConnectorDialInfo,
    resolver: &dyn DynSecretResolver,
    allow_internal_hosts: bool,
) -> Outcome {
    let parsed = match Dial::parse(adapter, &info.dial) {
        Ok(dial) => dial,
        Err(err) => return Outcome::misconfigured(format!("connector's dial is invalid: {err}")),
    };
    let target = match &parsed {
        Dial::Sql(dial) => DialTarget::from(dial),
        Dial::Cdc(dial) => DialTarget::from(dial),
        Dial::Files(_) | Dial::Rest(_) | Dial::Sheets(_) => {
            return Outcome::misconfigured(format!(
                "connector is misconfigured: its dial does not match its own adapter {adapter:?}"
            ));
        }
    };
    match target.driver {
        SqlDriver::Postgres => probe_postgres(info, resolver, allow_internal_hosts).await,
        SqlDriver::Mysql => {
            probe_mysql(&target, &info.secret_ref, resolver, allow_internal_hosts).await
        }
        SqlDriver::Mssql => {
            probe_mssql(&target, &info.secret_ref, resolver, allow_internal_hosts).await
        }
    }
}

/// Real connectivity probe for a `mysql`-driver `sql`/`cdc` dial (WS3 item
/// 13). `resolve_checked` runs FIRST, before `secret_ref` is even
/// resolved — see the module doc comment's "SSRF" section and this file's
/// `probe_mysql_blocks_an_internal_host_before_dialing` test, which hands
/// this function a resolver that panics if it is ever called, proving the
/// blocked path never reaches secret resolution let alone a dial.
async fn probe_mysql(
    target: &DialTarget<'_>,
    secret_ref: &str,
    resolver: &dyn DynSecretResolver,
    allow_internal_hosts: bool,
) -> Outcome {
    if let Err(message) = resolve_checked(target.host, target.port, allow_internal_hosts).await {
        return Outcome::misconfigured(message);
    }
    let password = match resolver.resolve_dyn(secret_ref).await {
        Ok(secret) => secret,
        Err(err) => {
            return Outcome::misconfigured(format!(
                "could not resolve the connector's credential: {err}"
            ));
        }
    };
    let options = MySqlConnectOptions::new()
        .host(target.host)
        .port(target.port)
        .username(target.user)
        .password(password.expose_secret())
        .database(target.database)
        // Same explicit-not-implicit reasoning as probe_postgres's
        // PgSslMode::Prefer: state the effective TLS posture rather than
        // leaving it to sqlx's default.
        .ssl_mode(MySqlSslMode::Preferred);

    let started = Instant::now();
    let attempt = tokio::time::timeout(DIAL_TIMEOUT, async {
        let mut conn = MySqlConnection::connect_with(&options).await?;
        sqlx::query("SELECT 1").execute(&mut conn).await?;
        conn.close().await
    })
    .await;
    let elapsed = started.elapsed();

    match attempt {
        Ok(Ok(())) => Outcome::success(elapsed, "Connected via MySQL and ran SELECT 1."),
        Ok(Err(err)) => Outcome::failure(
            elapsed,
            format!("MySQL connection failed: {}", classify_sqlx_error(&err)),
        ),
        Err(_) => Outcome::failure(
            elapsed,
            format!(
                "MySQL connection timed out after {}s",
                DIAL_TIMEOUT.as_secs()
            ),
        ),
    }
}

/// Classify a `tiberius` connection error into one of a small set of
/// generic failure classes — same discipline as `classify_sqlx_error`, see
/// the module doc comment's "Error messages never echo upstream data"
/// section.
///
/// `pub(crate)`: reused by `connector_discover` for its own SQL Server
/// discovery-connection failures (WS3 item 14) — see
/// `classify_sqlx_error`'s doc comment for the same reasoning.
pub(crate) fn classify_tiberius_error(err: &tiberius::error::Error) -> &'static str {
    match err {
        tiberius::error::Error::Io { kind, .. } => match kind {
            std::io::ErrorKind::ConnectionRefused => "connection refused",
            std::io::ErrorKind::TimedOut => "timed out",
            std::io::ErrorKind::PermissionDenied => "permission denied",
            _ => "connection failed",
        },
        tiberius::error::Error::Tls(_) => "TLS error",
        // SQL Server error 18456: "Login failed for user" -- the
        // documented code for an authentication rejection.
        tiberius::error::Error::Server(_) if err.code() == Some(18_456) => "authentication failed",
        tiberius::error::Error::Server(_) => "database rejected the connection",
        _ => "connection failed",
    }
}

/// Real connectivity probe for an `mssql`-driver `sql`/`cdc` dial (WS3
/// item 13), via the `tiberius` driver (`rust/crates/lakehouse-api/Cargo.toml`,
/// WS3 item 12). `resolve_checked` runs FIRST,
/// before `secret_ref` is resolved or a `TcpStream` is even opened — see
/// `probe_mssql_blocks_an_internal_host_before_dialing`, which mirrors
/// `probe_mysql`'s own SSRF test with a panicking resolver.
async fn probe_mssql(
    target: &DialTarget<'_>,
    secret_ref: &str,
    resolver: &dyn DynSecretResolver,
    allow_internal_hosts: bool,
) -> Outcome {
    if let Err(message) = resolve_checked(target.host, target.port, allow_internal_hosts).await {
        return Outcome::misconfigured(message);
    }
    let password = match resolver.resolve_dyn(secret_ref).await {
        Ok(secret) => secret,
        Err(err) => {
            return Outcome::misconfigured(format!(
                "could not resolve the connector's credential: {err}"
            ));
        }
    };

    let mut config = tiberius::Config::new();
    config.host(target.host);
    config.port(target.port);
    config.database(target.database);
    config.authentication(tiberius::AuthMethod::sql_server(
        target.user,
        password.expose_secret(),
    ));
    // Tier 1 has no cert-pinning story for an arbitrary operator-supplied
    // SQL Server host -- same "state the TLS posture explicitly" reasoning
    // as probe_postgres's PgSslMode::Prefer, applied to tiberius's own
    // "accept the presented certificate without verifying its chain"
    // knob.
    config.trust_cert();
    let addr = config.get_addr();

    let started = Instant::now();
    let attempt = tokio::time::timeout(DIAL_TIMEOUT, async move {
        let tcp = tokio::net::TcpStream::connect(&addr).await?;
        tcp.set_nodelay(true)?;
        let mut client = tiberius::Client::connect(config, tcp.compat_write()).await?;
        // A literal, constant query string -- never built from caller
        // input (tiberius's own `simple_query` doc comment: do not use
        // this with user-specified input).
        client
            .simple_query("SELECT 1")
            .await?
            .into_first_result()
            .await?;
        Ok::<(), tiberius::error::Error>(())
    })
    .await;
    let elapsed = started.elapsed();

    match attempt {
        Ok(Ok(())) => Outcome::success(elapsed, "Connected via SQL Server and ran SELECT 1."),
        Ok(Err(err)) => Outcome::failure(
            elapsed,
            format!(
                "SQL Server connection failed: {}",
                classify_tiberius_error(&err)
            ),
        ),
        Err(_) => Outcome::failure(
            elapsed,
            format!(
                "SQL Server connection timed out after {}s",
                DIAL_TIMEOUT.as_secs()
            ),
        ),
    }
}

/// Classify a `reqwest` error into one of a small set of generic failure
/// classes, using only its typed accessors (`is_connect`/`is_timeout`) —
/// never `Display`, which can echo request/response detail. See the
/// module doc comment's "Error messages never echo upstream data" section.
fn classify_reqwest_error(err: &reqwest::Error) -> &'static str {
    if err.is_connect() {
        "connection refused"
    } else if err.is_timeout() {
        "timed out"
    } else {
        "connection failed"
    }
}

fn probe_rest_misconfigured_dial(adapter: &str) -> Outcome {
    Outcome::misconfigured(format!(
        "connector is misconfigured: its dial does not match its own adapter {adapter:?}"
    ))
}

/// Parses `info.dial` as a `rest` adapter's [`RestDial`] and hands it to
/// [`probe_rest`] — the [`ConnectorDialInfo`]-facing entry point
/// [`probe`]'s dispatcher calls.
async fn probe_rest_info(
    info: &ConnectorDialInfo,
    resolver: &dyn DynSecretResolver,
    allow_internal_hosts: bool,
) -> Outcome {
    match Dial::parse("rest", &info.dial) {
        Ok(Dial::Rest(dial)) => {
            probe_rest(&dial, &info.secret_ref, resolver, allow_internal_hosts).await
        }
        Ok(_) => probe_rest_misconfigured_dial("rest"),
        Err(err) => Outcome::misconfigured(format!("connector's dial is invalid: {err}")),
    }
}

/// Real connectivity probe for a `rest` adapter's dial (WS3 item 13).
/// `resolve_checked` runs FIRST, against `dial.base_url`'s own
/// host/port, before any credential is resolved or request sent — see
/// `probe_rest_blocks_an_internal_host_before_dialing`.
async fn probe_rest(
    dial: &RestDial,
    secret_ref: &str,
    resolver: &dyn DynSecretResolver,
    allow_internal_hosts: bool,
) -> Outcome {
    let Some((host, port)) = parse_endpoint_host_port(&dial.base_url) else {
        return Outcome::misconfigured(
            "connector is misconfigured: a rest adapter's baseUrl must be an http(s):// URL",
        );
    };
    if let Err(message) = resolve_checked(host, port, allow_internal_hosts).await {
        return Outcome::misconfigured(message);
    }

    let request = reqwest::Client::new().get(&dial.base_url);
    let request = match &dial.auth {
        RestAuth::ApiKey { header } => {
            let key = match resolver.resolve_dyn(secret_ref).await {
                Ok(secret) => secret,
                Err(err) => {
                    return Outcome::misconfigured(format!(
                        "could not resolve the connector's credential: {err}"
                    ));
                }
            };
            request.header(header, key.expose_secret())
        }
        RestAuth::Bearer => {
            let token = match resolver.resolve_dyn(secret_ref).await {
                Ok(secret) => secret,
                Err(err) => {
                    return Outcome::misconfigured(format!(
                        "could not resolve the connector's credential: {err}"
                    ));
                }
            };
            request.bearer_auth(token.expose_secret())
        }
        RestAuth::Basic => {
            // The resolved secretRef is treated as the already-encoded
            // "user:pass" credential material for this header -- a `rest`
            // adapter's `ConnectorDialInfo` carries exactly one
            // `secret_ref`, not a separate username field, so the operator
            // creating the connector supplies the credential in the shape
            // this probe sends verbatim in the Authorization header.
            let basic = match resolver.resolve_dyn(secret_ref).await {
                Ok(secret) => secret,
                Err(err) => {
                    return Outcome::misconfigured(format!(
                        "could not resolve the connector's credential: {err}"
                    ));
                }
            };
            request.header(
                reqwest::header::AUTHORIZATION,
                format!("Basic {}", basic.expose_secret()),
            )
        }
        RestAuth::Oauth2ClientCredentials { .. } => {
            // Never fabricated: an OAuth2 client-credentials exchange is a
            // second network call this build does not implement in Tier 1
            // (mirrors probe_sheets's own "unsupported, honestly" posture
            // for a credential exchange this workspace cannot verify).
            return Outcome::misconfigured(
                "connector is misconfigured: OAuth2 client-credentials REST auth is not \
                 implemented by this build's connectivity probe -- configure apiKey or bearer \
                 auth to test this connector",
            );
        }
    };

    let started = Instant::now();
    let attempt = tokio::time::timeout(DIAL_TIMEOUT, request.send()).await;
    let elapsed = started.elapsed();

    match attempt {
        Ok(Ok(response)) if response.status().is_success() => Outcome::success(
            elapsed,
            "Connected via REST and received a successful response.",
        ),
        Ok(Ok(response)) => Outcome::failure(
            elapsed,
            format!("REST request rejected: HTTP {}", response.status().as_u16()),
        ),
        Ok(Err(err)) => Outcome::failure(
            elapsed,
            format!("REST request failed: {}", classify_reqwest_error(&err)),
        ),
        Err(_) => Outcome::failure(
            elapsed,
            format!("REST request timed out after {}s", DIAL_TIMEOUT.as_secs()),
        ),
    }
}

/// `sheets` ships `supported: false` unconditionally in Tier 1 (WS3 plan
/// review Z10, open question 4): a real Google service-account
/// JWT-bearer token exchange requires SIGNING a claim set with the
/// account's RSA private key, and no such signing path has been
/// implemented or verified against a real service account. `jsonwebtoken =
/// "9"` (`rust/Cargo.toml`) is already a workspace dependency, but
/// `lakehouse-auth`'s `oidc` module uses it to VERIFY tokens this API
/// receives, not to MINT a service-account assertion — mirrors
/// `dagster/dispar_orchestrate`'s own `adapters/sheets.py`, which ships
/// the SAME unconditional `supported: false` for the same reason on the
/// Dagster side. This function therefore never resolves a `secretRef` and
/// never opens a connection to `oauth2.googleapis.com`/
/// `sheets.googleapis.com` at all — "unsupported, honestly" (AGENTS.md
/// rule 14) over a crypto code path nobody has run against a real service
/// account, not a partial implementation that only sometimes dials.
fn probe_sheets() -> Outcome {
    Outcome {
        ok: false,
        supported: false,
        latency_ms: None,
        message: "This build cannot test a Google Sheets connector: no verified \
                   service-account token exchange exists yet."
            .to_owned(),
    }
}

/// A private/internal address this build refuses to dial by default — see
/// the module doc comment's "SSRF" section. Covers:
///
/// - RFC1918 IPv4 (`10/8`, `172.16/12`, `192.168/16`) via
///   [`std::net::Ipv4Addr::is_private`].
/// - Loopback (`127/8`, `::1`).
/// - Link-local (`169.254/16` — this is where cloud metadata services
///   (AWS/GCP/Azure instance metadata) live, `fe80::/10`).
/// - The unspecified address (`0.0.0.0`, `::`), which several TCP stacks
///   treat as "this host".
/// - IPv6 unique-local (`fc00::/7`).
/// - An IPv4-mapped IPv6 address (`::ffff:a.b.c.d`) whose embedded IPv4
///   address is itself any of the above — otherwise this whole check is
///   bypassable by asking DNS for an AAAA record wrapping a blocked IPv4
///   address.
fn is_blocked_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_private() || v4.is_loopback() || v4.is_link_local() || v4.is_unspecified()
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() || v6.is_unspecified() {
                return true;
            }
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return mapped.is_private()
                    || mapped.is_loopback()
                    || mapped.is_link_local()
                    || mapped.is_unspecified();
            }
            let first_segment = v6.segments()[0];
            let is_unique_local = first_segment & 0xfe00 == 0xfc00; // fc00::/7
            let is_link_local = first_segment & 0xffc0 == 0xfe80; // fe80::/10
            is_unique_local || is_link_local
        }
    }
}

/// Resolve `host:port` via DNS and refuse it if any resolved address is
/// private/internal (unless `allow_internal_hosts`) — see the module doc
/// comment's "SSRF" section for why this resolves rather than
/// pattern-matching the literal `host` string. Returns `Err` with a message
/// safe to surface directly (never includes upstream response data — there
/// is none at this stage, only DNS resolution).
///
/// `pub(crate)`, not private: `routes::connectors::ingest_spec_put` calls
/// this directly to run the SAME check as a second, non-authoritative
/// SSRF guard at `PUT .../ingest-spec` save time (WS3 plan judge review
/// Z1) — see that function's doc comment for why a save-time check can
/// only ever be advisory, not a substitute for this dial-time one.
pub(crate) async fn resolve_checked(
    host: &str,
    port: u16,
    allow_internal_hosts: bool,
) -> Result<(), String> {
    let addrs: Vec<std::net::SocketAddr> = match tokio::net::lookup_host((host, port)).await {
        Ok(iter) => iter.collect(),
        Err(err) => return Err(format!("could not resolve host {host:?}: {err}")),
    };
    if addrs.is_empty() {
        return Err(format!("host {host:?} did not resolve to any address"));
    }
    if allow_internal_hosts {
        return Ok(());
    }
    for addr in &addrs {
        if is_blocked_ip(&addr.ip()) {
            return Err(format!(
                "refusing to dial {host:?}: it resolves to {}, a private/internal address this \
                 build blocks by default (set CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS=true to \
                 allow this for a trusted internal deployment)",
                addr.ip()
            ));
        }
    }
    Ok(())
}

/// The pieces [`probe_postgres`] needs to build a [`PgConnectOptions`]
/// field-by-field, parsed from `host` shaped
/// `<user>@<host>:<port>/<database>` (`0022_prune_connector_seed.sql`'s
/// `conn-pg-lakehouse` row). This is NOT a DSN (it carries no password) —
/// [`probe_postgres`] combines it with the resolved credential in-memory,
/// for this dial only, and NEVER by interpolating strings into a
/// `postgres://` URL: `PgConnectOptions` fields are individually bound, so
/// a `host`/`database` value that smuggled `sslmode=disable`,
/// `options=...`, or a `?`-query string cannot reach `sqlx` as anything
/// other than a literal hostname/database name.
///
/// `pub(crate)`, not private: `crate::connector_deprovision` parses the
/// exact same `host` shape to build the `PgTarget` it dials to drop a
/// deleted connector's replication slot/publication — reusing this parser
/// rather than duplicating the `<user>@<host>:<port>/<database>` split
/// keeps there being exactly one place that shape is defined.
pub(crate) struct PgDialTarget<'a> {
    pub(crate) user: &'a str,
    pub(crate) host: &'a str,
    pub(crate) port: u16,
    pub(crate) database: &'a str,
}

pub(crate) fn parse_postgres_host(host: &str) -> Option<PgDialTarget<'_>> {
    let (user, rest) = host.split_once('@')?;
    if user.is_empty() {
        return None;
    }
    let (host_and_port, database) = rest.split_once('/')?;
    if database.is_empty() {
        return None;
    }
    let (host, port_str) = host_and_port.rsplit_once(':')?;
    if host.is_empty() {
        return None;
    }
    let port: u16 = port_str.parse().ok()?;
    Some(PgDialTarget {
        user,
        host,
        port,
        database,
    })
}

/// Classify a `sqlx` connection error into one of a small set of generic
/// failure classes — see the module doc comment's "Error messages never
/// echo upstream data" section for why this never formats `err` itself.
///
/// `pub(crate)`: `connector_discover` classifies its own Postgres/`MySQL`
/// discovery-connection failures through this SAME function (WS3 item
/// 14) rather than a second copy — the "never echo upstream data"
/// property must hold identically for both callers.
pub(crate) fn classify_sqlx_error(err: &sqlx::Error) -> &'static str {
    match err {
        sqlx::Error::Io(io_err) => match io_err.kind() {
            std::io::ErrorKind::ConnectionRefused => "connection refused",
            std::io::ErrorKind::TimedOut => "timed out",
            std::io::ErrorKind::PermissionDenied => "permission denied",
            _ => "connection failed",
        },
        sqlx::Error::Tls(_) => "TLS error",
        sqlx::Error::Database(db_err) => match db_err.code().as_deref() {
            // PostgreSQL error codes: 28P01 invalid_password,
            // 28000 invalid_authorization_specification.
            Some("28P01" | "28000") => "authentication failed",
            _ => "database rejected the connection",
        },
        sqlx::Error::PoolTimedOut => "timed out",
        _ => "connection failed",
    }
}

async fn probe_postgres(
    info: &ConnectorDialInfo,
    resolver: &dyn DynSecretResolver,
    allow_internal_hosts: bool,
) -> Outcome {
    let Some(target) = parse_postgres_host(&info.host) else {
        return Outcome::misconfigured(
            "connector is misconfigured: PostgreSQL host must be shaped \
             \"<user>@<host>:<port>/<database>\"",
        );
    };
    if let Err(message) = resolve_checked(target.host, target.port, allow_internal_hosts).await {
        return Outcome::misconfigured(message);
    }
    let password = match resolver.resolve_dyn(&info.secret_ref).await {
        Ok(secret) => secret,
        Err(err) => {
            return Outcome::misconfigured(format!(
                "could not resolve the connector's credential: {err}"
            ));
        }
    };
    let options = PgConnectOptions::new()
        .host(target.host)
        .port(target.port)
        .username(target.user)
        .password(password.expose_secret())
        .database(target.database)
        // Pinned explicitly rather than left to sqlx's default so the
        // effective TLS posture is a decision this module states, not an
        // artifact of whatever `sqlx` happens to default to. `Prefer`
        // (attempt TLS, fall back to plaintext) matches this deployment's
        // compose-network Postgres, which does not terminate TLS.
        .ssl_mode(PgSslMode::Prefer);

    let started = Instant::now();
    let attempt = tokio::time::timeout(DIAL_TIMEOUT, async {
        let mut conn = PgConnection::connect_with(&options).await?;
        sqlx::query("SELECT 1").execute(&mut conn).await?;
        conn.close().await
    })
    .await;
    let elapsed = started.elapsed();

    match attempt {
        Ok(Ok(())) => Outcome::success(elapsed, "Connected via PostgreSQL and ran SELECT 1."),
        Ok(Err(err)) => Outcome::failure(
            elapsed,
            format!(
                "PostgreSQL connection failed: {}",
                classify_sqlx_error(&err)
            ),
        ),
        Err(_) => Outcome::failure(
            elapsed,
            format!(
                "PostgreSQL connection timed out after {}s",
                DIAL_TIMEOUT.as_secs()
            ),
        ),
    }
}

/// Parses `host` as `<endpoint>|<bucket>` — the shape
/// `0022_prune_connector_seed.sql`'s `conn-s3-warehouse` row uses.
fn parse_s3_host(host: &str) -> Option<(&str, &str)> {
    let (endpoint, bucket) = host.split_once('|')?;
    if endpoint.is_empty() || bucket.is_empty() {
        return None;
    }
    Some((endpoint, bucket))
}

/// Extracts `(host, port)` from an `http://`/`https://` endpoint URL,
/// without pulling in a full URL-parsing dependency — `object_store`'s
/// `AmazonS3Builder::with_endpoint` only ever needs `host`/`port` from
/// this to be handed to [`resolve_checked`], not the whole URL structure.
///
/// `pub(crate)`, not private: `routes::connectors::ingest_spec_put` reuses
/// this to pull `host`/`port` out of a `files` adapter's `dial.endpoint`
/// and a `rest` adapter's `dial.baseUrl` for the same save-time SSRF check
/// [`resolve_checked`]'s doc comment describes — the two adapters whose
/// dial shape names a URL rather than a bare `host`/`port` pair.
pub(crate) fn parse_endpoint_host_port(endpoint: &str) -> Option<(&str, u16)> {
    let (authority, default_port) = if let Some(rest) = endpoint.strip_prefix("https://") {
        (rest, 443)
    } else if let Some(rest) = endpoint.strip_prefix("http://") {
        (rest, 80)
    } else {
        return None;
    };
    let authority = authority.split(['/', '?', '#']).next()?;
    if authority.is_empty() {
        return None;
    }
    match authority.rsplit_once(':') {
        Some((host, port_str)) => {
            let port: u16 = port_str.parse().ok()?;
            Some((host, port))
        }
        None => Some((authority, default_port)),
    }
}

/// Classify an `object_store` request error into one of a small set of
/// generic failure classes — see the module doc comment's "Error messages
/// never echo upstream data" section. Deliberately never formats `err`
/// itself: `object_store::Error::Generic`'s `Display` impl includes the
/// upstream HTTP response body verbatim, and this probe's `host` is
/// caller-controlled (see the module doc comment), so doing so would leak
/// arbitrary response bodies from wherever the caller pointed this probe.
fn classify_object_store_error(err: &object_store::Error) -> &'static str {
    match err {
        object_store::Error::NotFound { .. } => "not found",
        object_store::Error::PermissionDenied { .. } => "permission denied",
        object_store::Error::Unauthenticated { .. } => "authentication failed",
        object_store::Error::NotSupported { .. } => "not supported",
        _ => classify_via_reqwest_source(err),
    }
}

/// Walks an error's `source()` chain looking for a wrapped
/// [`reqwest::Error`] to classify connect/timeout failures more precisely.
/// Only ever reads `reqwest::Error`'s typed accessors (`is_connect`,
/// `is_timeout`) — never its `Display` text, which can itself carry
/// upstream detail.
fn classify_via_reqwest_source(err: &(dyn std::error::Error + 'static)) -> &'static str {
    let mut current: Option<&(dyn std::error::Error + 'static)> = err.source();
    while let Some(inner) = current {
        if let Some(reqwest_err) = inner.downcast_ref::<reqwest::Error>() {
            if reqwest_err.is_connect() {
                return "connection refused";
            }
            if reqwest_err.is_timeout() {
                return "timed out";
            }
            if reqwest_err.is_status() {
                return "request rejected by upstream";
            }
        }
        current = inner.source();
    }
    "connection failed"
}

async fn probe_s3(
    info: &ConnectorDialInfo,
    resolver: &dyn DynSecretResolver,
    allow_internal_hosts: bool,
) -> Outcome {
    let Some((endpoint, bucket)) = parse_s3_host(&info.host) else {
        return Outcome::misconfigured(
            "connector is misconfigured: S3 host must be shaped \"<endpoint>|<bucket>\"",
        );
    };
    let Some((endpoint_host, endpoint_port)) = parse_endpoint_host_port(endpoint) else {
        return Outcome::misconfigured(
            "connector is misconfigured: S3 endpoint must be an http(s):// URL",
        );
    };
    if let Err(message) = resolve_checked(endpoint_host, endpoint_port, allow_internal_hosts).await
    {
        return Outcome::misconfigured(message);
    }
    let Some(secret_ref_secondary) = info.secret_ref_secondary.as_deref() else {
        return Outcome::misconfigured(
            "connector is misconfigured: an S3 connector needs both secretRef (access key id) \
             and a secondary secretRef (secret access key)",
        );
    };
    let access_key = match resolver.resolve_dyn(&info.secret_ref).await {
        Ok(secret) => secret,
        Err(err) => {
            return Outcome::misconfigured(format!(
                "could not resolve the connector's access-key credential: {err}"
            ));
        }
    };
    let secret_key = match resolver.resolve_dyn(secret_ref_secondary).await {
        Ok(secret) => secret,
        Err(err) => {
            return Outcome::misconfigured(format!(
                "could not resolve the connector's secret-key credential: {err}"
            ));
        }
    };

    let built = AmazonS3Builder::new()
        .with_endpoint(endpoint)
        .with_bucket_name(bucket)
        .with_access_key_id(access_key.expose_secret())
        .with_secret_access_key(secret_key.expose_secret())
        // Self-hosted (RustFS), not real AWS S3: path-style addressing, and
        // `object_store` must be told explicitly this is not talking to
        // AWS or it refuses a plain-http/self-signed endpoint outright —
        // same posture `lakehouse-iceberg::storage`'s client uses.
        .with_virtual_hosted_style_request(false)
        .with_allow_http(true)
        .build();
    let client = match built {
        Ok(client) => client,
        Err(err) => {
            return Outcome::misconfigured(format!("failed to build the S3 client: {err}"));
        }
    };

    let started = Instant::now();
    let attempt = tokio::time::timeout(DIAL_TIMEOUT, client.list_with_delimiter(None)).await;
    let elapsed = started.elapsed();

    match attempt {
        Ok(Ok(_)) => Outcome::success(elapsed, "Connected via S3 and listed the bucket."),
        Ok(Err(err)) => Outcome::failure(
            elapsed,
            format!(
                "S3 connection failed: {}",
                classify_object_store_error(&err)
            ),
        ),
        Err(_) => Outcome::failure(
            elapsed,
            format!("S3 connection timed out after {}s", DIAL_TIMEOUT.as_secs()),
        ),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use lakehouse_core::secret::EnvSecretResolver;

    use super::*;

    fn info(
        kind: &str,
        host: &str,
        secret_ref: &str,
        secondary: Option<&str>,
    ) -> ConnectorDialInfo {
        ConnectorDialInfo {
            kind: kind.to_owned(),
            host: host.to_owned(),
            secret_ref: secret_ref.to_owned(),
            secret_ref_secondary: secondary.map(str::to_owned),
            // These tests exercise `probe`'s type-based dispatch, not the
            // adapter-based deprovision dispatch WS3 added — every case
            // here is a pre-WS3-shaped fixture (WS3 plan review X4).
            adapter: None,
            dial: serde_json::json!({}),
        }
    }

    #[tokio::test]
    async fn unsupported_kind_never_fabricates_a_latency_or_success() {
        let resolver = EnvSecretResolver::with_map(std::collections::HashMap::new());
        for kind in ["Kafka", "MQTT", "MongoDB", "Oracle", "SAP / ERP", "SFTP"] {
            let outcome = probe(&info(kind, "h", "env:X", None), &resolver, false).await;
            assert!(!outcome.supported, "{kind} must be unsupported");
            assert!(!outcome.ok, "{kind} must never report ok=true");
            assert!(
                outcome.latency_ms.is_none(),
                "{kind} must never report a latency"
            );
            assert!(
                outcome.message.contains(kind),
                "{kind}: {}",
                outcome.message
            );
        }
    }

    #[tokio::test]
    async fn postgres_malformed_host_is_misconfigured_not_a_panic() {
        let resolver = EnvSecretResolver::with_map(std::collections::HashMap::new());
        let outcome = probe(
            &info("PostgreSQL", "not-a-valid-host-shape", "env:X", None),
            &resolver,
            false,
        )
        .await;
        assert!(outcome.supported);
        assert!(!outcome.ok);
        assert!(outcome.latency_ms.is_none());
    }

    #[tokio::test]
    async fn postgres_unreachable_host_reports_real_elapsed_time_and_failure() {
        let mut map = std::collections::HashMap::new();
        map.insert("PG_TEST_PASSWORD".to_owned(), "irrelevant".to_owned());
        let resolver = EnvSecretResolver::with_map(map);
        // Port 1 on localhost: nothing listens there, so this fails fast
        // via connection-refused rather than exercising the 5s timeout —
        // keeps this test quick while still measuring a real elapsed time.
        // `allow_internal_hosts: true` — this test is specifically about
        // the dial-failure path, not the SSRF blocklist (covered
        // separately below), and 127.0.0.1 is itself blocked by default.
        let outcome = probe(
            &info(
                "PostgreSQL",
                "u@127.0.0.1:1/db",
                "env:PG_TEST_PASSWORD",
                None,
            ),
            &resolver,
            true,
        )
        .await;
        assert!(outcome.supported);
        assert!(!outcome.ok);
        assert!(outcome.latency_ms.is_some());
        assert!(outcome.message.contains("PostgreSQL connection failed"));
    }

    /// Blocker 2: a loopback host must be refused by default, before any
    /// dial is attempted — no latency, because no attempt was made.
    #[tokio::test]
    async fn postgres_loopback_host_is_blocked_by_default() {
        let mut map = std::collections::HashMap::new();
        map.insert("PG_TEST_PASSWORD".to_owned(), "irrelevant".to_owned());
        let resolver = EnvSecretResolver::with_map(map);
        let outcome = probe(
            &info(
                "PostgreSQL",
                "u@127.0.0.1:1/db",
                "env:PG_TEST_PASSWORD",
                None,
            ),
            &resolver,
            false,
        )
        .await;
        assert!(outcome.supported);
        assert!(!outcome.ok);
        assert!(
            outcome.latency_ms.is_none(),
            "a blocked host must never be dialed, so no latency exists"
        );
        assert!(
            outcome.message.contains("private/internal"),
            "{}",
            outcome.message
        );
    }

    /// A private RFC1918 host (not just loopback) must also be blocked.
    #[tokio::test]
    async fn postgres_rfc1918_host_is_blocked_by_default() {
        let mut map = std::collections::HashMap::new();
        map.insert("PG_TEST_PASSWORD".to_owned(), "irrelevant".to_owned());
        let resolver = EnvSecretResolver::with_map(map);
        let outcome = probe(
            &info(
                "PostgreSQL",
                "u@10.1.2.3:5432/db",
                "env:PG_TEST_PASSWORD",
                None,
            ),
            &resolver,
            false,
        )
        .await;
        assert!(!outcome.ok);
        assert!(outcome.latency_ms.is_none());
    }

    /// The documented opt-out: with `allow_internal_hosts: true`, a
    /// loopback/private host is no longer blocked and the probe proceeds
    /// to a real (here, failing) dial attempt — proving the flag actually
    /// takes effect, not just that the default blocks.
    #[tokio::test]
    async fn postgres_loopback_host_is_reachable_when_internal_hosts_allowed() {
        let mut map = std::collections::HashMap::new();
        map.insert("PG_TEST_PASSWORD".to_owned(), "irrelevant".to_owned());
        let resolver = EnvSecretResolver::with_map(map);
        let outcome = probe(
            &info(
                "PostgreSQL",
                "u@127.0.0.1:1/db",
                "env:PG_TEST_PASSWORD",
                None,
            ),
            &resolver,
            true,
        )
        .await;
        assert!(outcome.supported);
        assert!(!outcome.ok);
        assert!(
            outcome.latency_ms.is_some(),
            "allow_internal_hosts=true must let this reach the real dial attempt"
        );
        assert!(outcome.message.contains("PostgreSQL connection failed"));
    }

    #[tokio::test]
    async fn s3_missing_secondary_secret_ref_is_misconfigured() {
        let resolver = EnvSecretResolver::with_map(std::collections::HashMap::new());
        let outcome = probe(
            &info("Object storage", "http://127.0.0.1:1|bucket", "env:X", None),
            &resolver,
            true,
        )
        .await;
        assert!(outcome.supported);
        assert!(!outcome.ok);
        assert!(outcome.latency_ms.is_none());
    }

    #[tokio::test]
    async fn s3_unreachable_endpoint_reports_real_elapsed_time_and_failure() {
        let mut map = std::collections::HashMap::new();
        map.insert("AK".to_owned(), "ak".to_owned());
        map.insert("SK".to_owned(), "sk".to_owned());
        let resolver = EnvSecretResolver::with_map(map);
        let outcome = probe(
            &info(
                "Object storage",
                "http://127.0.0.1:1|bucket",
                "env:AK",
                Some("env:SK"),
            ),
            &resolver,
            true,
        )
        .await;
        assert!(outcome.supported);
        assert!(!outcome.ok);
        assert!(outcome.latency_ms.is_some());
    }

    /// Blocker 2 for the S3 probe: a loopback endpoint is blocked before
    /// any HTTP request is made.
    #[tokio::test]
    async fn s3_loopback_endpoint_is_blocked_by_default() {
        let mut map = std::collections::HashMap::new();
        map.insert("AK".to_owned(), "ak".to_owned());
        map.insert("SK".to_owned(), "sk".to_owned());
        let resolver = EnvSecretResolver::with_map(map);
        let outcome = probe(
            &info(
                "Object storage",
                "http://127.0.0.1:1|bucket",
                "env:AK",
                Some("env:SK"),
            ),
            &resolver,
            false,
        )
        .await;
        assert!(outcome.supported);
        assert!(!outcome.ok);
        assert!(outcome.latency_ms.is_none());
        assert!(outcome.message.contains("private/internal"));
    }

    /// A link-local address (169.254/16 — cloud metadata service range)
    /// must be blocked too, not just RFC1918/loopback.
    #[tokio::test]
    async fn s3_link_local_metadata_endpoint_is_blocked_by_default() {
        let mut map = std::collections::HashMap::new();
        map.insert("AK".to_owned(), "ak".to_owned());
        map.insert("SK".to_owned(), "sk".to_owned());
        let resolver = EnvSecretResolver::with_map(map);
        let outcome = probe(
            &info(
                "Object storage",
                "http://169.254.169.254|bucket",
                "env:AK",
                Some("env:SK"),
            ),
            &resolver,
            false,
        )
        .await;
        assert!(!outcome.ok);
        assert!(outcome.latency_ms.is_none());
    }

    #[test]
    fn is_blocked_ip_covers_every_documented_range() {
        let blocked = [
            "127.0.0.1",       // loopback
            "10.0.0.1",        // RFC1918
            "172.16.5.5",      // RFC1918
            "192.168.1.1",     // RFC1918
            "169.254.169.254", // link-local / cloud metadata
            "0.0.0.0",         // unspecified
            "::1",             // IPv6 loopback
            "fc00::1",         // IPv6 unique-local
            "fe80::1",         // IPv6 link-local
            "::ffff:10.0.0.1", // IPv4-mapped IPv6, private
        ];
        for ip in blocked {
            let addr: IpAddr = ip.parse().unwrap();
            assert!(is_blocked_ip(&addr), "{ip} should be blocked");
        }

        let allowed = [
            "8.8.8.8",
            "1.1.1.1",
            "2606:4700:4700::1111", // Cloudflare public IPv6
        ];
        for ip in allowed {
            let addr: IpAddr = ip.parse().unwrap();
            assert!(!is_blocked_ip(&addr), "{ip} should not be blocked");
        }
    }

    /// Blocker 2's core requirement: the check must apply to the RESOLVED
    /// address, not the literal `host` string — a hostname that resolves
    /// to a blocked address is blocked exactly like the literal address
    /// would be, so pointing DNS at an innocuous-looking name cannot
    /// bypass the filter.
    #[tokio::test]
    async fn resolve_checked_blocks_a_hostname_that_resolves_to_loopback() {
        let err = resolve_checked("localhost", 1, false).await.unwrap_err();
        assert!(err.contains("private/internal"), "{err}");
    }

    #[tokio::test]
    async fn resolve_checked_allows_a_hostname_when_internal_hosts_allowed() {
        resolve_checked("localhost", 1, true)
            .await
            .expect("allow_internal_hosts=true must let a loopback-resolving host through");
    }

    // -- WS3 item 13: probe_mysql/probe_mssql/probe_rest/probe_sheets --

    /// A resolver that panics if `resolve_dyn` is ever called — used to
    /// prove a blocked SSRF outcome never even reaches secret resolution,
    /// let alone a driver connection attempt.
    #[derive(Debug)]
    struct PanicIfCalledResolver;

    #[async_trait::async_trait]
    impl DynSecretResolver for PanicIfCalledResolver {
        async fn resolve_dyn(
            &self,
            _secret_ref: &str,
        ) -> Result<lakehouse_core::secret::SecretValue, lakehouse_core::secret::SecretError>
        {
            panic!(
                "resolve_checked must block an internal host BEFORE any secret is resolved, \
                 but resolve_dyn was called"
            );
        }
    }

    fn sql_dial(driver: SqlDriver, host: &str, port: u16) -> SqlDial {
        SqlDial {
            driver,
            host: host.to_owned(),
            port,
            database: "x".to_owned(),
            user: "u".to_owned(),
            ssl_mode: None,
        }
    }

    /// Mirrors `postgres_loopback_host_is_blocked_by_default`: a loopback
    /// host must be refused before `probe_mysql` ever touches
    /// `resolver`/a `MySqlConnection`.
    #[tokio::test]
    async fn probe_mysql_blocks_an_internal_host_before_dialing() {
        let dial = sql_dial(SqlDriver::Mysql, "127.0.0.1", 3306);
        let target = DialTarget::from(&dial);
        let outcome = probe_mysql(&target, "env:UNUSED", &PanicIfCalledResolver, false).await;
        assert!(!outcome.ok);
        assert!(outcome.supported);
        assert!(
            outcome.latency_ms.is_none(),
            "a blocked host must never be dialed, so no latency exists"
        );
        assert!(
            outcome.message.contains("private/internal"),
            "{}",
            outcome.message
        );
    }

    /// Same property as above, for `probe_mssql` — proves the guard sits
    /// before `tiberius::Config`/`TcpStream::connect` too, not only before
    /// `sqlx`.
    #[tokio::test]
    async fn probe_mssql_blocks_an_internal_host_before_dialing() {
        let dial = sql_dial(SqlDriver::Mssql, "127.0.0.1", 1433);
        let target = DialTarget::from(&dial);
        let outcome = probe_mssql(&target, "env:UNUSED", &PanicIfCalledResolver, false).await;
        assert!(!outcome.ok);
        assert!(outcome.supported);
        assert!(outcome.latency_ms.is_none());
        assert!(
            outcome.message.contains("private/internal"),
            "{}",
            outcome.message
        );
    }

    /// Same property for `probe_rest` — the guard runs against
    /// `dial.base_url`'s own host/port before any header is built or
    /// request sent.
    #[tokio::test]
    async fn probe_rest_blocks_an_internal_host_before_dialing() {
        let dial = RestDial {
            base_url: "http://127.0.0.1:1".to_owned(),
            auth: RestAuth::Bearer,
            pagination: lakehouse_store::ingest_spec::RestPagination::None,
            endpoints: vec![],
        };
        let outcome = probe_rest(&dial, "env:UNUSED", &PanicIfCalledResolver, false).await;
        assert!(!outcome.ok);
        assert!(outcome.supported);
        assert!(outcome.latency_ms.is_none());
        assert!(
            outcome.message.contains("private/internal"),
            "{}",
            outcome.message
        );
    }

    #[tokio::test]
    async fn probe_rest_reports_ok_on_a_2xx_response() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(wiremock::ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let dial = RestDial {
            base_url: server.uri(),
            // RestAuth has no "no auth" variant (ADR 0013) -- Bearer is
            // the smallest variant that exercises the resolver, with the
            // resolved token unused by wiremock's unconditional 200 mock.
            auth: RestAuth::Bearer,
            pagination: lakehouse_store::ingest_spec::RestPagination::None,
            endpoints: vec![],
        };
        // allow_internal_hosts: true -- wiremock's ephemeral server binds
        // loopback, and this test is about the 2xx-success path, not the
        // SSRF blocklist (covered separately above).
        let mut map = std::collections::HashMap::new();
        map.insert("REST_TEST_TOKEN".to_owned(), "irrelevant".to_owned());
        let resolver = EnvSecretResolver::with_map(map);
        let outcome = probe_rest(&dial, "env:REST_TEST_TOKEN", &resolver, true).await;
        assert!(outcome.ok, "{}", outcome.message);
        assert!(outcome.supported);
        assert!(outcome.latency_ms.is_some());
    }

    #[tokio::test]
    async fn probe_rest_reports_the_status_code_never_the_body_on_a_4xx_response() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(
                wiremock::ResponseTemplate::new(403).set_body_string("secret upstream detail"),
            )
            .mount(&server)
            .await;
        let dial = RestDial {
            base_url: server.uri(),
            auth: RestAuth::Bearer,
            pagination: lakehouse_store::ingest_spec::RestPagination::None,
            endpoints: vec![],
        };
        let mut map = std::collections::HashMap::new();
        map.insert("REST_TEST_TOKEN".to_owned(), "irrelevant".to_owned());
        let resolver = EnvSecretResolver::with_map(map);
        let outcome = probe_rest(&dial, "env:REST_TEST_TOKEN", &resolver, true).await;
        assert!(!outcome.ok);
        assert!(outcome.supported);
        assert!(outcome.message.contains("403"));
        assert!(
            !outcome.message.contains("secret upstream detail"),
            "the response body must never be echoed: {}",
            outcome.message
        );
    }

    /// `sheets` never dials anything, in any state -- proves it reports
    /// `supported: false` even though this test never seeds a
    /// `secret_ref` at all, matching Tier 1's unconditional posture (see
    /// `probe_sheets`'s doc comment).
    #[test]
    fn probe_sheets_is_unconditionally_unsupported() {
        let outcome = probe_sheets();
        assert!(!outcome.ok);
        assert!(!outcome.supported);
        assert!(outcome.latency_ms.is_none());
    }
}
