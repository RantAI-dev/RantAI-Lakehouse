//! `POST /api/connectors/{id}/discover` — list a connector's source
//! objects (tables/columns) so a caller can pick what to ingest, without
//! first hand-typing table names into an ingest spec.
//!
//! # The property this module exists to protect (WS3 item 14)
//!
//! Every SQL-flavoured discovery query takes a caller-supplied schema
//! name (the connector's own configured schema, echoed back through the
//! `?schema=` query parameter) and must bind it, never interpolate it
//! into the query text — the schema name reaches this module exactly the
//! way any other caller-influenced string would, and `format!`-ing it
//! into SQL would be the textbook injection shape `AGENTS.md` forbids
//! ("SQL values bound, `format!` only for constant identifiers").
//! [`POSTGRES_DISCOVER_QUERY`]/[`MYSQL_DISCOVER_QUERY`]/
//! [`MSSQL_DISCOVER_QUERY`] are `const`, each with exactly one
//! driver-appropriate bind placeholder (`$1`/`?`/`@P1`) and no
//! interpolation site at all — `discover_sql_queries_bind_the_schema_never_interpolate_it`
//! (below) asserts this on the query TEXT itself, for all three drivers.
//!
//! # How MySQL/SQL Server discovery is tested today (`SRC-9`, finding G1)
//!
//! This workspace's `#[sqlx::test]` macro and its `testcontainers` fixture
//! (`lakehouse-test-support`, `postgres` module only) stand up a scratch
//! Postgres database; there is no `MySQL` or SQL Server live-database
//! fixture inside `cargo test`. [`discover_sql_postgres`] therefore has a
//! real, live `#[sqlx::test]` proof. [`discover_sql_mysql`] and
//! [`discover_sql_mssql`] are exercised live by the `ops/g6` gate
//! (`_check_discover`, `SRC-9`), against its `mysql`/`mssql` compose services.
//! That gate's first CI run found that `MySQL` discovery answered `422`
//! ("connection failed") while the same connector's connection test passed:
//! the connect options are the same function, so the failure was in the
//! query's row decoding (see [`MYSQL_DISCOVER_QUERY`]). Whether that fix is
//! enough is decided by the next g6 run.
//!
//! # Scope: `sql`/`cdc` only in this task
//!
//! `files`/`rest`/`sheets` adapters answer with an honest
//! `supported: false` here — same "unsupported, honestly" discipline
//! `connector_probe::probe_sheets` already uses for the `sheets` adapter
//! (AGENTS.md rule 14) — rather than a half-built `ListObjectsV2`/JSON
//! walk added under this task's schema-binding-focused scope. Real
//! discovery for those adapters is follow-up work, tracked outside this
//! task; nothing here claims otherwise.

use lakehouse_core::ApiError;
use lakehouse_core::secret::DynSecretResolver;
use lakehouse_store::connectors::ConnectorDialInfo;
use lakehouse_store::ingest_spec::{Dial, IngestSpecError, SqlDriver};
use sqlx::mysql::MySqlPoolOptions;
use sqlx::postgres::PgPoolOptions;
use tokio_util::compat::TokioAsyncWriteCompatExt;

use crate::connector_probe::{self, DIAL_TIMEOUT, DialTarget};
use crate::internal_hosts::InternalHosts;

/// One discovered column.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredColumn {
    /// Column name, as reported by the source database's own
    /// `information_schema`.
    pub name: String,
    /// The source database's own type name (e.g. `"integer"`, `"text"`) —
    /// passed through verbatim, never normalized to a `lakehouse` type.
    pub type_name: String,
}

/// One discovered table (or, for a future non-SQL adapter, an equivalent
/// object) — `name` is `"<schema>.<table>"`, matching the `SourceObject.name`
/// shape (`lakehouse_store::ingest_spec::SourceObject`), so a caller can copy
/// it directly into an ingest spec's `sourceObjects`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredObject {
    /// `"<schema>.<table>"`.
    pub name: String,
    /// This table's columns, in the source database's own ordinal order.
    pub columns: Vec<DiscoveredColumn>,
}

/// The response body for `POST /api/connectors/{id}/discover`. `reason`
/// is populated whenever `supported` is `false` — never left implicit.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverResult {
    /// Every discovered table, empty when `supported` is `false`.
    pub objects: Vec<DiscoveredObject>,
    /// Whether this build knows how to discover this connector's
    /// adapter/schema at all.
    pub supported: bool,
    /// Populated whenever `supported` is `false`; `None` otherwise.
    pub reason: Option<String>,
}

impl DiscoverResult {
    fn unsupported(reason: impl Into<String>) -> Self {
        Self {
            objects: Vec::new(),
            supported: false,
            reason: Some(reason.into()),
        }
    }

    fn ok(objects: Vec<DiscoveredObject>) -> Self {
        Self {
            objects,
            supported: true,
            reason: None,
        }
    }
}

/// Everything that can go wrong opening a discovery connection or running
/// the discovery query itself. Never carries raw upstream error text past
/// its `source` (diagnosability only, via `#[source]`) — the route
/// handler classifies each variant into the same fixed, safe-to-surface
/// message classes `connector_probe` already uses, never this type's own
/// `Display` (see that module's "Error messages never echo upstream
/// data" doc section).
#[derive(Debug, thiserror::Error)]
pub enum DiscoverError {
    /// A `sqlx`-driven query (Postgres or `MySQL`) failed, at connect or
    /// query time — `sqlx` does not distinguish the two in its own error
    /// type, so neither does this variant.
    #[error("discovery query failed: {0}")]
    Sql(#[from] sqlx::Error),
    /// A `tiberius`-driven (SQL Server) query failed.
    #[error("discovery query failed: {0}")]
    Tiberius(#[from] tiberius::error::Error),
    /// The connector's referenced credential could not be resolved.
    #[error("could not resolve the connector's credential: {0}")]
    Secret(String),
    /// The connector's stored `dial` does not parse as the shape its own
    /// `adapter` names.
    #[error("connector's dial is invalid: {0}")]
    InvalidDial(#[from] IngestSpecError),
    /// The dial's `host` resolves to (or is) a private/internal address
    /// that `internal_hosts` does not permit — see
    /// `connector_probe::resolve_checked`.
    #[error("{0}")]
    Blocked(String),
    /// The connect-then-query attempt did not finish within
    /// [`DIAL_TIMEOUT`].
    #[error("discovery timed out after {}s", DIAL_TIMEOUT.as_secs())]
    Timeout,
}

/// Turn a [`DiscoverError`] into the response the route handler returns —
/// 422 (the connector's own configuration/target is the problem, not this
/// server): a malformed `dial`, an SSRF-blocked host, an unresolved
/// credential, or the discovery connection/query itself failing. Every
/// arm classifies its `#[source]` through `connector_probe`'s existing
/// classifiers rather than this type's own `Display` — `sqlx::Error` and
/// `tiberius::error::Error` can both echo connection-string/response
/// fragments in their raw text (see `connector_probe`'s module doc
/// comment, "Error messages never echo upstream data"), and this is the
/// SAME discipline `AGENTS.md`'s known-gap note requires (never another
/// `ApiError::Internal(err.to_string())`).
impl From<DiscoverError> for ApiError {
    fn from(err: DiscoverError) -> Self {
        // `SRC-9` finding G1: the handler used to log nothing about why a
        // discovery failed, so a decode error and a refused connection were
        // indistinguishable ("connection failed" in the response, nothing in
        // the log). Log the variant and, for `sqlx`, its error kind only.
        // Never `err`'s text: a `sqlx` database error echoes the server's
        // message, which can name the user and host.
        if let DiscoverError::Sql(source) = &err {
            tracing::warn!(
                sqlx_kind = sqlx_error_kind(source),
                class = connector_probe::classify_sqlx_error(source),
                "connector discovery failed"
            );
        }
        match err {
            DiscoverError::Sql(source) => ApiError::Unprocessable(format!(
                "could not discover the connector's schema: {}",
                connector_probe::classify_sqlx_error(&source)
            )),
            DiscoverError::Tiberius(source) => ApiError::Unprocessable(format!(
                "could not discover the connector's schema: {}",
                connector_probe::classify_tiberius_error(&source)
            )),
            DiscoverError::Secret(message) => ApiError::Unprocessable(format!(
                "could not resolve the connector's credential: {message}"
            )),
            DiscoverError::InvalidDial(source) => {
                ApiError::Unprocessable(format!("connector's dial is invalid: {source}"))
            }
            DiscoverError::Blocked(message) => ApiError::Unprocessable(message),
            DiscoverError::Timeout => ApiError::Unprocessable(format!(
                "discovery timed out after {}s",
                DIAL_TIMEOUT.as_secs()
            )),
        }
    }
}

/// The variant name of a `sqlx::Error`, for the server log only. A fixed
/// string per variant: no upstream text, so it cannot carry a credential.
fn sqlx_error_kind(err: &sqlx::Error) -> &'static str {
    match err {
        sqlx::Error::Configuration(_) => "Configuration",
        sqlx::Error::Database(_) => "Database",
        sqlx::Error::Io(_) => "Io",
        sqlx::Error::Tls(_) => "Tls",
        sqlx::Error::Protocol(_) => "Protocol",
        sqlx::Error::RowNotFound => "RowNotFound",
        sqlx::Error::TypeNotFound { .. } => "TypeNotFound",
        sqlx::Error::ColumnIndexOutOfBounds { .. } => "ColumnIndexOutOfBounds",
        sqlx::Error::ColumnNotFound(_) => "ColumnNotFound",
        sqlx::Error::ColumnDecode { .. } => "ColumnDecode",
        sqlx::Error::Decode(_) => "Decode",
        sqlx::Error::PoolTimedOut => "PoolTimedOut",
        sqlx::Error::PoolClosed => "PoolClosed",
        sqlx::Error::WorkerCrashed => "WorkerCrashed",
        _ => "Other",
    }
}

/// Group `(table_name, column_name, data_type)` rows — already ordered by
/// `(table_name, ordinal_position)` by every driver's own query — into one
/// [`DiscoveredObject`] per distinct table, preserving column order.
fn group_columns_by_table(
    schema: &str,
    rows: Vec<(String, String, String)>,
) -> Vec<DiscoveredObject> {
    let mut objects: Vec<DiscoveredObject> = Vec::new();
    for (table_name, column_name, data_type) in rows {
        let full_name = format!("{schema}.{table_name}");
        match objects.last_mut() {
            Some(last) if last.name == full_name => {
                last.columns.push(DiscoveredColumn {
                    name: column_name,
                    type_name: data_type,
                });
            }
            _ => objects.push(DiscoveredObject {
                name: full_name,
                columns: vec![DiscoveredColumn {
                    name: column_name,
                    type_name: data_type,
                }],
            }),
        }
    }
    objects
}

/// `$1`: `sqlx`'s Postgres positional-parameter placeholder syntax.
pub(crate) const POSTGRES_DISCOVER_QUERY: &str = "SELECT table_name, column_name, data_type FROM \
    information_schema.columns WHERE table_schema = $1 ORDER BY table_name, ordinal_position";

/// List every table and column in `schema` on a live Postgres connection.
/// `schema` is bound as `$1` — see the module doc comment's "The property
/// this module exists to protect".
///
/// # Errors
///
/// Returns [`DiscoverError::Sql`] if the query fails.
pub async fn discover_sql_postgres(
    pool: &sqlx::PgPool,
    schema: &str,
) -> Result<Vec<DiscoveredObject>, DiscoverError> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(POSTGRES_DISCOVER_QUERY)
        .bind(schema)
        .fetch_all(pool)
        .await
        .map_err(DiscoverError::from)?;
    Ok(group_columns_by_table(schema, rows))
}

// `information_schema.columns` is the SAME standard SQL view `MySQL`
// exposes (`MySQL` 8's own `information_schema.columns`) — the query
// shape is identical to Postgres's, only the placeholder syntax (`?`,
// `MySQL`'s positional-parameter convention) differs.
//
// `SRC-9` finding G1: the g6 gate showed `MySQL` discovery failing with the
// generic "connection failed" class while the connection test (same
// `mysql_connect_options`) passed. `MySQL` 8 reports some
// `information_schema` columns with a binary collation / `VARBINARY`-like
// type, which `sqlx` refuses to decode as `String` (`ColumnDecode`, which
// `classify_sqlx_error` folds into "connection failed"). Casting every
// selected column to `CHAR` with an explicit charset makes the decode
// independent of the server's column type. Still `const`, with exactly one
// bind placeholder. Not proven by a live test here; the g6 gate decides.
pub(crate) const MYSQL_DISCOVER_QUERY: &str = "SELECT \
    CAST(table_name AS CHAR CHARACTER SET utf8mb4), \
    CAST(column_name AS CHAR CHARACTER SET utf8mb4), \
    CAST(data_type AS CHAR CHARACTER SET utf8mb4) \
    FROM information_schema.columns WHERE table_schema = ? \
    ORDER BY table_name, ordinal_position";

/// List every table and column in `schema` on a live `MySQL` connection.
/// `schema` is bound as `?` — see the module doc comment.
///
/// # Errors
///
/// Returns [`DiscoverError::Sql`] if the query fails.
pub async fn discover_sql_mysql(
    pool: &sqlx::MySqlPool,
    schema: &str,
) -> Result<Vec<DiscoveredObject>, DiscoverError> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(MYSQL_DISCOVER_QUERY)
        .bind(schema)
        .fetch_all(pool)
        .await
        .map_err(DiscoverError::from)?;
    Ok(group_columns_by_table(schema, rows))
}

// SQL Server's own `INFORMATION_SCHEMA.COLUMNS` (T-SQL is
// case-insensitive for identifiers by default) — `tiberius` (not `sqlx`)
// uses `@P1`-numbered parameters, not `?`/`$1`.
pub(crate) const MSSQL_DISCOVER_QUERY: &str = "SELECT table_name, column_name, data_type FROM \
    information_schema.columns WHERE table_schema = @P1 ORDER BY table_name, ordinal_position";

/// List every table and column in `schema` on a live SQL Server
/// connection. `schema` is bound as `@P1` — see the module doc comment.
///
/// # Errors
///
/// Returns [`DiscoverError::Tiberius`] if the query fails.
pub async fn discover_sql_mssql(
    client: &mut tiberius::Client<tokio_util::compat::Compat<tokio::net::TcpStream>>,
    schema: &str,
) -> Result<Vec<DiscoveredObject>, DiscoverError> {
    let stream = client
        .query(MSSQL_DISCOVER_QUERY, &[&schema])
        .await
        .map_err(DiscoverError::from)?;
    let mut rows = Vec::new();
    for row in stream
        .into_first_result()
        .await
        .map_err(DiscoverError::from)?
    {
        rows.push((
            row.get::<&str, _>(0).unwrap_or_default().to_owned(),
            row.get::<&str, _>(1).unwrap_or_default().to_owned(),
            row.get::<&str, _>(2).unwrap_or_default().to_owned(),
        ));
    }
    Ok(group_columns_by_table(schema, rows))
}

/// The fixed reason for Oracle, which the console cannot discover (`SRC-9`
/// finding G2, `SRC-13`).
pub(crate) const ORACLE_DISCOVER_REASON: &str = "Oracle tables are listed by the orchestrator, not \
    the console; console discovery for Oracle is tracked as SRC-13";

/// Open a bounded, SSRF-checked discovery connection for `target` and list
/// `schema`, dispatching on `target.driver` — the SAME
/// `resolve_checked`-before-`secret_ref`-before-dial ordering
/// `connector_probe::probe_dial` uses (never skip the address check, even
/// for a read-only discovery call: the SSRF exposure is identical).
async fn discover_dial(
    target: &DialTarget<'_>,
    secret_ref: &str,
    schema: &str,
    resolver: &dyn DynSecretResolver,
    internal_hosts: &InternalHosts,
) -> Result<Vec<DiscoveredObject>, DiscoverError> {
    let approved = connector_probe::resolve_checked(target.host, target.port, internal_hosts)
        .await
        .map_err(DiscoverError::Blocked)?;
    let password = resolver
        .resolve_dyn(secret_ref)
        .await
        .map_err(|err| DiscoverError::Secret(err.to_string()))?;

    let attempt = tokio::time::timeout(DIAL_TIMEOUT, async {
        match target.driver {
            SqlDriver::Postgres => {
                let options = connector_probe::pg_connect_options(
                    &approved,
                    target.user,
                    password.expose_secret(),
                    target.database,
                );
                let pool = PgPoolOptions::new()
                    .max_connections(1)
                    .connect_with(options)
                    .await?;
                let objects = discover_sql_postgres(&pool, schema).await?;
                pool.close().await;
                Ok(objects)
            }
            SqlDriver::Mysql => {
                let options = connector_probe::mysql_connect_options(
                    &approved,
                    target.user,
                    password.expose_secret(),
                    target.database,
                );
                let pool = MySqlPoolOptions::new()
                    .max_connections(1)
                    .connect_with(options)
                    .await?;
                let objects = discover_sql_mysql(&pool, schema).await?;
                pool.close().await;
                Ok(objects)
            }
            SqlDriver::Mssql => {
                let mut config = tiberius::Config::new();
                config.host(target.host);
                config.port(target.port);
                config.database(target.database);
                config.authentication(tiberius::AuthMethod::sql_server(
                    target.user,
                    password.expose_secret(),
                ));
                // Same "state the TLS posture explicitly" reasoning as
                // `connector_probe::probe_mssql`.
                config.trust_cert();
                // SEC-15: the socket goes to the approved address;
                // `config.host` keeps the name for the TDS handshake.
                // `TcpStream::connect`/`set_nodelay` return `std::io::Error`,
                // not `tiberius::error::Error` — routed through
                // `tiberius::error::Error::from` explicitly (a single `?`
                // only auto-converts one hop, and `DiscoverError` converts
                // from `tiberius::error::Error`, not from `io::Error`
                // directly) rather than adding a second `#[from]` variant
                // for an error class `connector_probe::probe_mssql` already
                // folds into its own `tiberius::error::Error::Io` handling.
                let tcp = connector_probe::connect_pinned(&approved)
                    .await
                    .map_err(tiberius::error::Error::from)?;
                let mut client =
                    Box::pin(tiberius::Client::connect(config, tcp.compat_write())).await?;
                discover_sql_mssql(&mut client, schema).await
            }
            SqlDriver::Oracle => {
                // Oracle schema discovery runs through the Dagster sql
                // adapter (`dagster/dispar_orchestrate/adapters/oracle.py`
                // over `oracledb` thin mode), not through this Rust-side
                // probe: `sqlx` and `tiberius` cannot speak to it. The arm
                // keeps `SqlDriver` exhaustive and is never dialed.
                // `SRC-9` finding G2: this used to answer `Blocked` (HTTP
                // 422) while every other type the API cannot discover
                // answers `supported: false`; `discover` now intercepts
                // Oracle before this function, so this arm is unreachable
                // from it and refuses defensively if it is ever reached.
                Err(DiscoverError::Blocked(ORACLE_DISCOVER_REASON.to_owned()))
            }
        }
    })
    .await;

    match attempt {
        Ok(result) => result,
        Err(_) => Err(DiscoverError::Timeout),
    }
}

/// Discover `schema`'s objects for `dial_info`, dispatching on
/// `dial_info.adapter`. `schema` is required for the `sql`/`cdc`
/// adapters (the caller-supplied string every driver's discovery query
/// binds — see the module doc comment) and ignored otherwise.
///
/// # Errors
///
/// Returns [`DiscoverError`] if a `sql`/`cdc` adapter's dial fails to
/// parse, its host is SSRF-blocked, its credential fails to resolve, or
/// the discovery connection/query itself fails.
pub async fn discover(
    dial_info: &ConnectorDialInfo,
    schema: Option<&str>,
    resolver: &dyn DynSecretResolver,
    internal_hosts: &InternalHosts,
) -> Result<DiscoverResult, DiscoverError> {
    match dial_info.adapter.as_deref() {
        Some(adapter @ ("sql" | "cdc")) => {
            let Some(schema) = schema else {
                return Ok(DiscoverResult::unsupported(
                    "the \"schema\" query parameter is required for a sql/cdc connector",
                ));
            };
            let parsed = Dial::parse(adapter, &dial_info.dial)?;
            let target = match &parsed {
                Dial::Sql(dial) => DialTarget::from(dial),
                Dial::Cdc(dial) => DialTarget::from(dial),
                Dial::Files(_)
                | Dial::Rest(_)
                | Dial::Sheets(_)
                | Dial::Mongo(_)
                | Dial::Kafka(_)
                | Dial::Sftp(_) => {
                    return Err(DiscoverError::Blocked(format!(
                        "connector is misconfigured: its dial does not match its own adapter \
                         {adapter:?}"
                    )));
                }
            };
            // `SRC-9` finding G2: an honest `supported: false`, like every
            // other type the API cannot discover, and before any SSRF check
            // or credential resolution because nothing will be dialed.
            let is_oracle = match &parsed {
                Dial::Sql(dial) => dial.driver == SqlDriver::Oracle,
                Dial::Cdc(dial) => dial.driver == SqlDriver::Oracle,
                _ => false,
            };
            if is_oracle {
                return Ok(DiscoverResult::unsupported(ORACLE_DISCOVER_REASON));
            }
            let objects = discover_dial(
                &target,
                &dial_info.secret_ref,
                schema,
                resolver,
                internal_hosts,
            )
            .await?;
            Ok(DiscoverResult::ok(objects))
        }
        Some("files") => Ok(DiscoverResult::unsupported(
            "object listing for a files connector is not implemented in this build",
        )),
        Some("rest") => Ok(DiscoverResult::unsupported(
            "endpoint discovery for a rest connector is not implemented in this build",
        )),
        Some("sheets") => Ok(DiscoverResult::unsupported("no service account configured")),
        None => Ok(DiscoverResult::unsupported(
            "connector has no ingest-spec adapter configured yet",
        )),
        Some(other) => Ok(DiscoverResult::unsupported(format!(
            "unknown adapter {other:?}"
        ))),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, reason = "test-only module")]
mod tests {
    use super::{
        MSSQL_DISCOVER_QUERY, MYSQL_DISCOVER_QUERY, ORACLE_DISCOVER_REASON, POSTGRES_DISCOVER_QUERY,
    };
    use crate::internal_hosts::InternalHosts;
    use lakehouse_core::secret::{DynSecretResolver, SecretError, SecretValue};
    use lakehouse_store::connectors::ConnectorDialInfo;

    /// Fails the test if discovery touches the credential: an Oracle answer
    /// must come before any dial or secret resolution.
    #[derive(Debug)]
    struct PanicIfCalledResolver;

    #[async_trait::async_trait]
    impl DynSecretResolver for PanicIfCalledResolver {
        async fn resolve_dyn(&self, _secret_ref: &str) -> Result<SecretValue, SecretError> {
            panic!("Oracle discovery must not resolve a secret");
        }
    }

    /// `SRC-9` finding G2: Oracle answers `supported: false` with a fixed
    /// reason, like every other type the API cannot discover, not a 422.
    /// No earlier test asserted the 422.
    #[tokio::test]
    async fn oracle_discovery_is_an_honest_unsupported_answer() {
        let info = ConnectorDialInfo {
            kind: "Oracle".to_owned(),
            host: "oracle.example".to_owned(),
            secret_ref: "env:UNUSED".to_owned(),
            secret_ref_secondary: None,
            adapter: Some("sql".to_owned()),
            dial: serde_json::json!({
                "driver": "oracle",
                "host": "oracle.example",
                "port": 1521,
                "database": "FREEPDB1",
                "user": "u",
            }),
        };
        let result = super::discover(
            &info,
            Some("APP"),
            &PanicIfCalledResolver,
            &InternalHosts::NONE,
        )
        .await
        .expect("an unsupported answer, not an error");
        assert!(!result.supported);
        assert!(result.objects.is_empty());
        assert_eq!(result.reason.as_deref(), Some(ORACLE_DISCOVER_REASON));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn discover_sql_postgres_lists_columns_from_information_schema(pool: sqlx::PgPool) {
        sqlx::query("CREATE TABLE probe_target (id INT, name TEXT)")
            .execute(&pool)
            .await
            .expect("create fixture table");
        let objects = super::discover_sql_postgres(&pool, "public")
            .await
            .expect("discovery query succeeds");
        let table = objects
            .iter()
            .find(|o| o.name == "public.probe_target")
            .expect("probe_target listed");
        assert!(
            table
                .columns
                .iter()
                .any(|c| c.name == "id" && c.type_name == "integer")
        );
        assert!(
            table
                .columns
                .iter()
                .any(|c| c.name == "name" && c.type_name == "text")
        );
    }

    /// A pure test on the query TEXT each driver's discovery function
    /// issues — proves the schema name is a bind placeholder ($1/?/@P1),
    /// never `format!`'d into the string, for all three drivers this task
    /// adds. `MySQL`/`MSSQL` have no live-database `cargo test` fixture in
    /// this workspace (Z14) — this is the coverage that stands in for
    /// them at the Rust level.
    #[test]
    fn discover_sql_queries_bind_the_schema_never_interpolate_it() {
        assert!(POSTGRES_DISCOVER_QUERY.contains("$1"));
        assert!(!POSTGRES_DISCOVER_QUERY.contains("{schema}"));
        assert!(MYSQL_DISCOVER_QUERY.contains('?'));
        assert!(!MYSQL_DISCOVER_QUERY.contains("{schema}"));
        assert!(MSSQL_DISCOVER_QUERY.contains("@P1"));
        assert!(!MSSQL_DISCOVER_QUERY.contains("{schema}"));
    }
}
