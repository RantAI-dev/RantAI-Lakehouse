//! Reads the facts the setup checks of a change-capture source need
//! (`SRC-4` slice 4, decision S4-f; the checks themselves are
//! [`crate::cdc_setup_check`]).
//!
//! # Same posture as the connection test
//!
//! Each driver is dialled the way [`crate::connector_probe`] dials it, after the
//! same checks: [`resolve_checked`] runs first (the SSRF gate, `SEC-15`), the
//! socket goes to the address that was approved, and the credential is resolved
//! only afterwards. `PostgreSQL` goes through [`crate::pg_probe`]'s own client
//! (D10): it refuses every sign-in but `SCRAM-SHA-256` and requires TLS before
//! the startup packet, which `sqlx` cannot be told to do.
//!
//! # Fixed statements, no value, no server text
//!
//! Every statement here is a constant of this build. A value to compare (the
//! publication name, the selected tables) is compared in Rust by the pure
//! checks, against what the fixed statement returned. A statement the login may
//! not run, or a connection that ends, leaves its fact unread: the check is then
//! `unchecked`, with a fixed sentence. The server's error is dropped, not
//! formatted (principle 4).
//!
//! # It never fails the test
//!
//! The checks are advice about the source. They are read after the connection
//! test passed and never change its outcome; a connector can be saved whatever
//! they say.

use std::path::Path;
use std::time::Duration;

use lakehouse_core::secret::DynSecretResolver;
use lakehouse_store::connectors::{ConnectorDialInfo, SetupCheck};
use lakehouse_store::ingest_spec::{CdcDial, Dial, SqlDriver};
use sqlx::Connection;
use sqlx::mysql::MySqlConnection;
use tokio_util::compat::TokioAsyncWriteCompatExt;

use crate::cdc_setup_check::{
    AgentState, MssqlFacts, MysqlFacts, PG_STATEMENTS, PgFacts, mssql_checks, mysql_checks,
    postgres_checks,
};
use crate::connector_probe::{
    PgDialTarget, apply_mssql_tls, connect_pinned, mysql_connect_options, resolve_checked,
};
use crate::internal_hosts::InternalHosts;

/// The whole read, connection included. Longer than one dial
/// ([`crate::connector_probe::DIAL_TIMEOUT`]) because it runs several
/// statements; bounded so the connection test never waits on it unbounded.
const SETUP_TIMEOUT: Duration = Duration::from_secs(10);

const MYSQL_LOG_BIN: &str = "SELECT CAST(@@log_bin AS CHAR)";
const MYSQL_BINLOG_FORMAT: &str = "SELECT CAST(@@binlog_format AS CHAR)";
const MYSQL_BINLOG_ROW_IMAGE: &str = "SELECT CAST(@@binlog_row_image AS CHAR)";
const MYSQL_GRANTS: &str = "SHOW GRANTS FOR CURRENT_USER()";
const MYSQL_EXPIRE: &str = "SELECT CAST(@@binlog_expire_logs_seconds AS CHAR)";
const MYSQL_EXPIRE_DAYS: &str = "SELECT CAST(@@expire_logs_days AS CHAR)";
const MYSQL_CURRENT_USER: &str = "SELECT CURRENT_USER()";

const MSSQL_CDC_ENABLED: &str = "SELECT is_cdc_enabled FROM sys.databases WHERE name = DB_NAME()";
const MSSQL_TABLES: &str = "SELECT s.name, t.name, t.is_tracked_by_cdc FROM sys.tables t \
    JOIN sys.schemas s ON s.schema_id = t.schema_id";
const MSSQL_AGENT: &str =
    "SELECT status_desc FROM sys.dm_server_services WHERE servicename LIKE 'SQL Server Agent%'";

/// The setup checks of change-capture connector `info`, for the tables its spec
/// selects. Empty for a connector that is not change capture, whose dial does
/// not parse, or whose driver has no check in this build (`Oracle`: its change
/// capture, and so its check, is slice 6's).
pub async fn setup_checks(
    info: &ConnectorDialInfo,
    resolver: &dyn DynSecretResolver,
    internal_hosts: &InternalHosts,
    ca: Option<&Path>,
    tables: &[String],
) -> Vec<SetupCheck> {
    if info.adapter.as_deref() != Some("cdc") {
        return Vec::new();
    }
    let Ok(Dial::Cdc(dial)) = Dial::parse("cdc", &info.dial) else {
        return Vec::new();
    };
    match dial.driver {
        SqlDriver::Postgres => {
            let facts = pg_facts(&dial, info, resolver, internal_hosts, ca).await;
            postgres_checks(
                &facts,
                &dial.user,
                dial.publication_name.as_deref().unwrap_or_default(),
                tables,
            )
        }
        SqlDriver::Mysql => {
            let facts = mysql_facts(&dial, info, resolver, internal_hosts).await;
            mysql_checks(&facts, &dial.user)
        }
        SqlDriver::Mssql => {
            let facts = mssql_facts(&dial, info, resolver, internal_hosts, ca).await;
            mssql_checks(&facts, tables)
        }
        SqlDriver::Oracle => Vec::new(),
    }
}

async fn pg_facts(
    dial: &CdcDial,
    info: &ConnectorDialInfo,
    resolver: &dyn DynSecretResolver,
    internal_hosts: &InternalHosts,
    ca: Option<&Path>,
) -> PgFacts {
    let Ok(approved) = resolve_checked(&dial.host, dial.port, internal_hosts).await else {
        return PgFacts::default();
    };
    let Ok(password) = resolver.resolve_dyn(&info.secret_ref).await else {
        return PgFacts::default();
    };
    let target = PgDialTarget {
        user: &dial.user,
        host: &dial.host,
        port: dial.port,
        database: &dial.database,
    };
    let read = tokio::time::timeout(
        SETUP_TIMEOUT,
        crate::pg_probe::connect_and_fetch(&target, &approved, &password, ca, &PG_STATEMENTS),
    )
    .await;
    let Ok(Ok(answers)) = read else {
        return PgFacts::default();
    };
    let mut answers = answers.into_iter();
    PgFacts {
        wal_level: answers.next().flatten(),
        replication: answers.next().flatten(),
        publications: answers.next().flatten(),
        published: answers.next().flatten(),
    }
}

/// One `MySQL` statement's first cell, as text; `None` when it fails or
/// returns nothing.
async fn mysql_scalar(conn: &mut MySqlConnection, sql: &'static str) -> Option<String> {
    sqlx::query_scalar::<sqlx::MySql, Option<String>>(sql)
        .fetch_one(&mut *conn)
        .await
        .ok()
        .flatten()
}

async fn mysql_facts(
    dial: &CdcDial,
    info: &ConnectorDialInfo,
    resolver: &dyn DynSecretResolver,
    internal_hosts: &InternalHosts,
) -> MysqlFacts {
    let Ok(approved) = resolve_checked(&dial.host, dial.port, internal_hosts).await else {
        return MysqlFacts::default();
    };
    let Ok(password) = resolver.resolve_dyn(&info.secret_ref).await else {
        return MysqlFacts::default();
    };
    let options = mysql_connect_options(
        &approved,
        &dial.user,
        password.expose_secret(),
        &dial.database,
    );
    let read = tokio::time::timeout(SETUP_TIMEOUT, async {
        let mut conn = MySqlConnection::connect_with(&options).await.ok()?;
        let facts = MysqlFacts {
            log_bin: mysql_scalar(&mut conn, MYSQL_LOG_BIN).await,
            binlog_format: mysql_scalar(&mut conn, MYSQL_BINLOG_FORMAT).await,
            binlog_row_image: mysql_scalar(&mut conn, MYSQL_BINLOG_ROW_IMAGE).await,
            grants: sqlx::query_scalar::<sqlx::MySql, String>(MYSQL_GRANTS)
                .fetch_all(&mut conn)
                .await
                .ok(),
            expire_seconds: mysql_scalar(&mut conn, MYSQL_EXPIRE).await,
            // Slice 4 review SHOULD-FIX 6: the variable is gone on 8.4, which reads as 0.
            expire_days: Some(
                mysql_scalar(&mut conn, MYSQL_EXPIRE_DAYS)
                    .await
                    .unwrap_or_else(|| "0".to_owned()),
            ),
            current_user: mysql_scalar(&mut conn, MYSQL_CURRENT_USER).await,
        };
        let _ = conn.close().await;
        Some(facts)
    })
    .await;
    read.ok().flatten().unwrap_or_default()
}

async fn mssql_facts(
    dial: &CdcDial,
    info: &ConnectorDialInfo,
    resolver: &dyn DynSecretResolver,
    internal_hosts: &InternalHosts,
    ca: Option<&Path>,
) -> MssqlFacts {
    let Ok(approved) = resolve_checked(&dial.host, dial.port, internal_hosts).await else {
        return MssqlFacts::default();
    };
    let Ok(password) = resolver.resolve_dyn(&info.secret_ref).await else {
        return MssqlFacts::default();
    };
    let mut config = tiberius::Config::new();
    config.host(&dial.host);
    config.port(dial.port);
    config.database(&dial.database);
    config.authentication(tiberius::AuthMethod::sql_server(
        &dial.user,
        password.expose_secret(),
    ));
    apply_mssql_tls(&mut config, ca);
    let read = tokio::time::timeout(SETUP_TIMEOUT, async move {
        // SEC-15: the socket goes to the approved address; `config.host` keeps
        // the configured name for the TDS handshake and its certificate check.
        let tcp = connect_pinned(&approved).await.ok()?;
        let mut client = Box::pin(tiberius::Client::connect(config, tcp.compat_write()))
            .await
            .ok()?;
        Some(read_mssql(&mut client).await)
    })
    .await;
    read.ok().flatten().unwrap_or_default()
}

type MssqlClient = tiberius::Client<tokio_util::compat::Compat<tokio::net::TcpStream>>;

/// The rows of a constant statement, or `None` when it is refused or fails.
async fn mssql_rows(client: &mut MssqlClient, sql: &'static str) -> Option<Vec<tiberius::Row>> {
    client
        .simple_query(sql)
        .await
        .ok()?
        .into_first_result()
        .await
        .ok()
}

async fn read_mssql(client: &mut MssqlClient) -> MssqlFacts {
    let cdc_enabled = mssql_rows(client, MSSQL_CDC_ENABLED)
        .await
        .and_then(|rows| rows.first()?.try_get::<bool, _>(0).ok().flatten());
    let tables = mssql_rows(client, MSSQL_TABLES).await.map(|rows| {
        rows.iter()
            .filter_map(|row| {
                let schema = row.try_get::<&str, _>(0).ok().flatten()?;
                let table = row.try_get::<&str, _>(1).ok().flatten()?;
                let tracked = row.try_get::<bool, _>(2).ok().flatten().unwrap_or(false);
                Some((schema.to_owned(), table.to_owned(), tracked))
            })
            .collect()
    });
    // Needs VIEW SERVER STATE, which the documented login does not have: a
    // refusal leaves the state unread, never "running" (decision S4-f).
    let agent = match mssql_rows(client, MSSQL_AGENT).await {
        None => AgentState::Unread,
        // No row at all means not listed (Express, some platforms), not stopped
        // (Slice 4 review SHOULD-FIX 7).
        Some(rows) if rows.is_empty() => AgentState::NotListed,
        Some(rows) => {
            let running = rows.iter().any(|row| {
                row.try_get::<&str, _>(0)
                    .ok()
                    .flatten()
                    .is_some_and(|status| status.eq_ignore_ascii_case("Running"))
            });
            if running {
                AgentState::Running
            } else {
                AgentState::NotRunning
            }
        }
    };
    MssqlFacts {
        cdc_enabled,
        tables,
        agent,
    }
}
