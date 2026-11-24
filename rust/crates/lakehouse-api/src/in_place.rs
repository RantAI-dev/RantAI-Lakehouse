//! The in-place external catalog: what each engine can read, and a bounded,
//! injection-safe way to ask that engine which external catalogs, schemas,
//! tables and columns exist (`BI-44` slice 2).
//!
//! This module is a single source of truth for two facts that used to live in
//! prose only ([`docs/core/specs/bi-44.md`]):
//!
//! 1. the **source coverage matrix** — the engines and source types a
//!    `bi-only` installation can actually read in place. A source outside
//!    the matrix answers `supported: false` with an honest sentence; it is
//!    never silently registered as if the active engine could read it
//!    (`AGENTS.md` principle 2).
//! 2. **introspection** of the active engine's external tables. Every query
//!    here is bounded, every identifier is validated before it is quoted,
//!    and every upstream error is classified through [`upstream_error`] so
//!    raw engine text reaches only the log, never a response
//!    (`AGENTS.md` principle 4, SEC-11).
//!
//! [`docs/core/specs/bi-44.md`]: ../../../docs/core/specs/bi-44.md

use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_trino::{TrinoClient, TrinoError};
use serde::Serialize;
use serde_json::Value;

use crate::config::QueryEngine;
use crate::upstream_error::{self, Class, FailedAs};

/// What one engine can read in place for one source type. The `reason` is
/// `Some` only for an unsupported source, and is an honest sentence naming
/// the engine and what it can read instead — never an invented success.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSupport {
    /// Whether the active engine can read this source type in place.
    pub supported: bool,
    /// The engine the answer is about, spelled as the deployment reports it.
    pub engine: QueryEngine,
    /// The normalized source type the answer is about.
    pub source_type: String,
    /// Why an unsupported source is unsupported. `None` when `supported`.
    pub reason: Option<String>,
}

/// The source types `ClickHouse` reads through its external table engines
/// (`docs/core/specs/bi-44.md`'s matrix).
const CH_SOURCES: &[&str] = &["postgresql", "mysql", "clickhouse"];

/// The source types `Trino` reads through federated catalogs
/// (`docs/core/specs/bi-44.md`'s matrix).
const TRINO_SOURCES: &[&str] = &[
    "postgresql",
    "mysql",
    "mariadb",
    "mssql",
    "oracle",
    "clickhouse",
    "snowflake",
    "bigquery",
    "redshift",
];

/// Normalizes an operator- or console-supplied source type: trimmed,
/// lowercased, with the spellings the console is known to send folded to
/// the matrix's canonical names.
///
/// Spaces are removed as well as dashes so `SQL Server` and `sql-server`
/// reach the `mssql` alias. Only a fixed set of aliases is mapped — an
/// unknown name stays unknown and is reported as unsupported, never
/// guessed into a supported one.
fn normalize(source_type: &str) -> String {
    let raw: String = source_type
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_')
        .collect();
    match raw.as_str() {
        "postgres" | "postgresql" => "postgresql".to_owned(),
        "sqlserver" | "mssql" | "sqlservers" => "mssql".to_owned(),
        "mariadb" => "mariadb".to_owned(),
        other => other.to_owned(),
    }
}

/// The complete source coverage matrix for one engine. Callers use this
/// rather than keeping a second list in the console or the connector route.
#[must_use]
pub fn supported_source_types(engine: QueryEngine) -> &'static [&'static str] {
    match engine {
        QueryEngine::ClickHouse => CH_SOURCES,
        QueryEngine::Trino => TRINO_SOURCES,
    }
}

/// The honest answer for `engine` reading `source_type`, after normalization.
///
/// Every unsupported answer names the engine and points at the matrix's
/// supported list instead of claiming a capability the engine does not
/// have. No source is silently assumed to work.
#[must_use]
pub fn source_support(engine: QueryEngine, source_type: &str) -> SourceSupport {
    let normalized = normalize(source_type);
    let supported = supported_source_types(engine).contains(&normalized.as_str());
    let reason = if supported {
        None
    } else {
        let engine_name = engine.name();
        let supported = supported_source_types(engine).join(", ");
        let wider = match engine {
            QueryEngine::ClickHouse => {
                " `Trino` covers the broader federated source list, but it is not installed here"
            }
            QueryEngine::Trino => "",
        };
        Some(format!(
            "source type {normalized:?} is not supported in place by `{engine_name}`; \
             supported source types here are: {supported}{wider}"
        ))
    };
    SourceSupport {
        supported,
        engine,
        source_type: normalized,
        reason,
    }
}

/// One external table the active engine can read in place. `catalog` is
/// `Some` only for a 3-part `Trino` name; a `ClickHouse` database is the
/// schema. `full_name` is the identifier a caller writes in SQL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InPlaceTable {
    /// The federated catalog, for a 3-part `Trino` name.
    pub catalog: Option<String>,
    /// The schema (a `ClickHouse` database).
    pub schema: String,
    /// The table's own name.
    pub name: String,
    /// The dotted identifier a query writes.
    pub full_name: String,
    /// `trino` or `clickhouse`, the engine that answered.
    pub engine: &'static str,
}

/// One column of an external table, with the engine's own type spelling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InPlaceColumn {
    /// The column name.
    pub name: String,
    /// The type as the engine reports it.
    pub data_type: String,
}

/// A classified introspection failure. The raw upstream text is in the log
/// under a reference id; the `Display` of this error is a fixed sentence
/// followed by that id, so it is safe to put in a response.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct IntrospectionError(upstream_error::UpstreamFailure);

/// The fixed sentences introspection errors are reported with.
pub const INTROSPECTION: upstream_error::Context = upstream_error::Context::new(
    "in-place catalog introspection",
    "The external catalog could not be inspected.",
    "The query engine is unavailable.",
);

impl IntrospectionError {
    /// The HTTP error this classified failure becomes. The fixed message and
    /// its reference are built inside [`upstream_error`], so no route ever
    /// renders an upstream error's own text (SEC-11).
    #[must_use]
    pub fn into_api_error(self) -> lakehouse_core::ApiError {
        self.0.into_api_error(FailedAs::ServiceUnavailable)
    }

    /// Classifies a `Trino` error: a query or HTTP-status failure is the
    /// engine answering with a refusal, everything else (transport,
    /// timeout, row cap) is an engine that could not serve the request.
    fn from_trino(err: &TrinoError) -> Self {
        let class = match err {
            TrinoError::Query(_) | TrinoError::HttpStatus(_) | TrinoError::TooManyRows { .. } => {
                Class::Failed
            }
            TrinoError::Transport(_) | TrinoError::Timeout => Class::Unavailable,
        };
        Self(upstream_error::report(&INTROSPECTION, class, &err.to_string()))
    }

    /// Classifies a `ClickHouse` error with this crate's one classifier.
    fn from_ch(err: &ChError) -> Self {
        Self(upstream_error::report_ch(&INTROSPECTION, err))
    }

    /// A rejected identifier is this module's own validation failure, not an
    /// upstream error: the raw identifier never reaches a log or response.
    fn invalid_identifier() -> Self {
        Self(upstream_error::report(
            &INTROSPECTION,
            Class::Failed,
            &"invalid external catalog identifier",
        ))
    }
}

/// The user header used for introspection. This is a fixed internal identity,
/// never derived from request input, so a caller cannot choose which `Trino`
/// user the metadata reads run as.
const INTROSPECTION_USER: &str = "catalog-introspection";

/// Catalogs `Trino` ships with but which hold no customer data. Listing
/// `system`'s or `jmx`'s tables as sources would fabricate an external
/// catalog that is not one.
const TRINO_INTERNAL_CATALOGS: &[&str] = &[
    "system", "jmx", "memory", "tpch", "tpcds", "iceberg", "information_schema",
];

/// Schemas every SQL engine reserves. They hold metadata, not external
/// tables, and are never offered as in-place sources.
const INTERNAL_SCHEMAS: &[&str] = &["information_schema", "pg_catalog"];

/// Databases a `ClickHouse` instance uses for its own bookkeeping or
/// temporary tables, never for an external source in a `bi-only` install.
const CH_INTERNAL_DATABASES: &[&str] = &[
    "system",
    "information_schema",
    "INFORMATION_SCHEMA",
    "lakehouse_internal",
    "default",
];

/// Quotes one `Trino` identifier: control characters are refused and an
/// embedded double quote is doubled, the only escaping `Trino`'s grammar
/// accepts. A catalog, schema or table name from the engine can therefore
/// never break out of its quoted identifier.
fn trino_ident(raw: &str) -> Result<String, IntrospectionError> {
    if raw.is_empty() || raw.chars().any(char::is_control) {
        return Err(IntrospectionError::invalid_identifier());
    }
    Ok(format!("\"{}\"", raw.replace('"', "\"\"")))
}

/// Quotes one `ClickHouse` identifier under a strict allowlist: a name that
/// is not a plain word is refused rather than guessed into SQL. The names
/// here come from `system.tables`, but fail closed is still the right
/// posture for anything interpolated into a statement.
fn ch_ident(raw: &str) -> Result<String, IntrospectionError> {
    if raw.is_empty()
        || !raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(IntrospectionError::invalid_identifier());
    }
    Ok(format!("`{raw}`"))
}

/// The string in column zero of each result row. Introspection queries are
/// single-column `SHOW` statements, so a row with no string in column zero
/// is skipped rather than turned into a fabricated identifier.
fn first_column_strings(result: &lakehouse_trino::TrinoResult) -> Vec<String> {
    result
        .rows
        .iter()
        .filter_map(|row| row.first().and_then(Value::as_str).map(str::to_owned))
        .collect()
}

/// Lists the external tables of the active `Trino` installation, bounded to
/// `max_tables` entries across all catalogs and schemas.
///
/// # Errors
///
/// Returns [`IntrospectionError`] when `Trino` cannot run a `SHOW` statement,
/// or when an identifier it returned is not safe to quote. The raw engine
/// text is logged under a reference, never placed in this error's message.
pub async fn introspect_trino_tables(
    client: &TrinoClient,
    max_tables: usize,
) -> Result<Vec<InPlaceTable>, IntrospectionError> {
    if max_tables == 0 {
        return Ok(Vec::new());
    }
    let catalogs = first_column_strings(
        &client
            .run_statement("SHOW CATALOGS", INTROSPECTION_USER)
            .await
            .map_err(|err| IntrospectionError::from_trino(&err))?,
    );
    let mut out: Vec<InPlaceTable> = Vec::new();
    for catalog in catalogs {
        if out.len() >= max_tables {
            break;
        }
        if TRINO_INTERNAL_CATALOGS.contains(&catalog.to_ascii_lowercase().as_str()) {
            continue;
        }
        let quoted_catalog = trino_ident(&catalog)?;
        let schemas = first_column_strings(
            &client
                .run_statement(
                    &format!("SHOW SCHEMAS FROM {quoted_catalog}"),
                    INTROSPECTION_USER,
                )
                .await
                .map_err(|err| IntrospectionError::from_trino(&err))?,
        );
        for schema in schemas {
            if out.len() >= max_tables {
                break;
            }
            if INTERNAL_SCHEMAS.contains(&schema.to_ascii_lowercase().as_str()) {
                continue;
            }
            let quoted_schema = trino_ident(&schema)?;
            let tables = first_column_strings(
                &client
                    .run_statement(
                        &format!("SHOW TABLES FROM {quoted_catalog}.{quoted_schema}"),
                        INTROSPECTION_USER,
                    )
                    .await
                    .map_err(|err| IntrospectionError::from_trino(&err))?,
            );
            for table in tables {
                if out.len() >= max_tables {
                    break;
                }
                // Validate before the name is used anywhere: a value that
                // cannot be safely echoed into a later DESCRIBE is not a
                // source this installation can honestly offer.
                trino_ident(&table)?;
                out.push(InPlaceTable {
                    catalog: Some(catalog.clone()),
                    schema: schema.clone(),
                    name: table.clone(),
                    full_name: format!("{catalog}.{schema}.{table}"),
                    engine: "trino",
                });
            }
        }
    }
    Ok(out)
}

/// The columns of one 3-part `Trino` table, via `DESCRIBE`. The first two
/// columns of the answer are `Column` and `Type`; anything else is ignored.
///
/// # Errors
///
/// Returns [`IntrospectionError`] when `Trino` refuses the `DESCRIBE`, or an
/// identifier is not safe to quote. Upstream text is classified before it
/// reaches this error.
pub async fn introspect_trino_columns(
    client: &TrinoClient,
    catalog: &str,
    schema: &str,
    table: &str,
) -> Result<Vec<InPlaceColumn>, IntrospectionError> {
    let sql = format!(
        "DESCRIBE {}.{}.{}",
        trino_ident(catalog)?,
        trino_ident(schema)?,
        trino_ident(table)?
    );
    let result = client
        .run_statement(&sql, INTROSPECTION_USER)
        .await
        .map_err(|err| IntrospectionError::from_trino(&err))?;
    Ok(result
        .rows
        .iter()
        .filter_map(|row| {
            let name = row.first()?.as_str()?.to_owned();
            let data_type = row.get(1)?.as_str()?.to_owned();
            Some(InPlaceColumn { name, data_type })
        })
        .collect())
}

/// Lists the external or user tables of a `bi-only` `ClickHouse` instance,
/// bounded to `max_tables` entries. Internal databases are excluded; an
/// external table engine (`PostgreSQL`, `MySQL`, `MaterializedPostgreSQL`,
/// `MaterializedMySQL`) is included by virtue of living in a non-internal
/// database, so an operator who points one at their own source sees it here.
///
/// No Bronze/Silver/serving registry is dialed: those are lakehouse tables
/// and a `bi-only` install does not have them.
///
/// # Errors
///
/// Returns [`IntrospectionError`] when `ClickHouse` refuses the `system`
/// read. The raw engine text is logged under a reference only.
pub async fn introspect_clickhouse_external_tables(
    ch: &ChClient,
    max_tables: usize,
) -> Result<Vec<InPlaceTable>, IntrospectionError> {
    if max_tables == 0 {
        return Ok(Vec::new());
    }
    // The bound is an integer literal produced from a usize and the exclusion
    // list is this module's fixed constant, never caller text; every value
    // read from the engine is bound into the response, not interpolated into
    // a later statement.
    let excluded = CH_INTERNAL_DATABASES
        .iter()
        .map(|db| lakehouse_core::ident::SqlLiteral::from(*db).to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT database, name, engine FROM system.tables \
         WHERE database NOT IN ({excluded}) \
         ORDER BY database, name LIMIT {max_tables}"
    );
    let rows = ch
        .rows(&sql, None)
        .await
        .map_err(|err| IntrospectionError::from_ch(&err))?;
    let mut out: Vec<InPlaceTable> = Vec::with_capacity(rows.len().min(max_tables));
    for row in rows {
        if out.len() >= max_tables {
            break;
        }
        let Some(database) = row.get("database").and_then(Value::as_str) else {
            continue;
        };
        // Defense in depth: the SQL excludes these, but a deployment whose
        // engine answers a wider query must not make an internal database
        // look like an external source either.
        if CH_INTERNAL_DATABASES.contains(&database) {
            continue;
        }
        let Some(name) = row.get("name").and_then(Value::as_str) else {
            continue;
        };
        ch_ident(database)?;
        ch_ident(name)?;
        out.push(InPlaceTable {
            catalog: None,
            schema: database.to_owned(),
            name: name.to_owned(),
            full_name: format!("{database}.{name}"),
            engine: "clickhouse",
        });
    }
    Ok(out)
}

/// The columns of one `ClickHouse` external/user table, via `system.columns`.
/// Every literal is bound through [`SqlLiteral`](lakehouse_core::ident::SqlLiteral)
/// and the identifiers are validated before they are used.
///
/// # Errors
///
/// Returns [`IntrospectionError`] when `ClickHouse` refuses the read, or an
/// identifier is not safe to quote.
pub async fn introspect_clickhouse_columns(
    ch: &ChClient,
    database: &str,
    table: &str,
) -> Result<Vec<InPlaceColumn>, IntrospectionError> {
    ch_ident(database)?;
    ch_ident(table)?;
    let sql = format!(
        "SELECT name, type FROM system.columns WHERE database={} AND table={} ORDER BY position",
        lakehouse_core::ident::SqlLiteral::from(database),
        lakehouse_core::ident::SqlLiteral::from(table),
    );
    let rows = ch
        .rows(&sql, None)
        .await
        .map_err(|err| IntrospectionError::from_ch(&err))?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            let name = row.get("name")?.as_str()?.to_owned();
            let data_type = row.get("type")?.as_str()?.to_owned();
            Some(InPlaceColumn { name, data_type })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use serde_json::json;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// A `Trino` statement answer carrying exactly one page of rows.
    fn trino_page(columns: &[&str], rows: &[Vec<&str>]) -> Value {
        json!({
            "id": "q1",
            "columns": columns.iter().map(|n| json!({"name": n, "type": "varchar"})).collect::<Vec<_>>(),
            "data": rows,
        })
    }

    async fn mount_trino(server: &MockServer, contains: &str, body: Value) {
        Mock::given(method("POST"))
            .and(path("/v1/statement"))
            .and(body_string_contains(contains))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(server)
            .await;
    }

    fn trino_client(server: &MockServer) -> TrinoClient {
        TrinoClient::new(lakehouse_trino::TrinoConfig::new(server.uri()))
    }

    fn ch_client(server: &MockServer) -> ChClient {
        ChClient::new(server.uri(), "test".to_owned(), String::new())
    }

    #[test]
    fn clickhouse_matrix_supports_only_its_three_external_engines() {
        assert_eq!(
            supported_source_types(QueryEngine::ClickHouse),
            &["postgresql", "mysql", "clickhouse"]
        );
        for source in ["postgresql", "mysql", "clickhouse"] {
            let answer = source_support(QueryEngine::ClickHouse, source);
            assert!(answer.supported, "{source}: {answer:?}");
            assert_eq!(answer.engine, QueryEngine::ClickHouse);
            assert!(answer.reason.is_none());
        }
        for source in ["oracle", "mssql", "snowflake", "mongodb"] {
            let answer = source_support(QueryEngine::ClickHouse, source);
            assert!(!answer.supported, "{source}");
            let reason = answer.reason.as_deref().unwrap_or_default();
            let lower = reason.to_ascii_lowercase();
            assert!(lower.contains("clickhouse"), "{reason}");
            assert!(lower.contains("trino"), "{reason}");
        }
    }

    #[test]
    fn trino_matrix_supports_the_nine_federated_sources() {
        assert_eq!(
            supported_source_types(QueryEngine::Trino),
            &[
                "postgresql",
                "mysql",
                "mariadb",
                "mssql",
                "oracle",
                "clickhouse",
                "snowflake",
                "bigquery",
                "redshift",
            ]
        );
        for source in supported_source_types(QueryEngine::Trino) {
            assert!(source_support(QueryEngine::Trino, source).supported, "{source}");
        }
        for source in ["mongodb", "sftp", "kafka"] {
            let answer = source_support(QueryEngine::Trino, source);
            assert!(!answer.supported, "{source}");
            let reason = answer.reason.as_deref().unwrap_or_default();
            assert!(reason.contains("trino"), "{reason}");
        }
    }

    #[test]
    fn source_type_aliases_and_case_are_normalized() {
        assert_eq!(
            source_support(QueryEngine::ClickHouse, "  PostgreSQL  ").source_type,
            "postgresql"
        );
        assert_eq!(
            source_support(QueryEngine::Trino, "SQL Server").source_type,
            "mssql"
        );
        assert!(source_support(QueryEngine::Trino, "mssql").supported);
        assert!(source_support(QueryEngine::Trino, "Postgres").supported);
        assert!(source_support(QueryEngine::ClickHouse, "MySQL").supported);
    }

    #[test]
    fn an_unsupported_source_serializes_with_camel_case_and_a_reason() {
        let value = serde_json::to_value(source_support(QueryEngine::ClickHouse, "oracle")).unwrap();
        assert_eq!(value["supported"], json!(false));
        assert_eq!(value["engine"], json!("clickhouse"));
        assert_eq!(value["sourceType"], json!("oracle"));
        assert!(value["reason"].as_str().unwrap().contains("not supported in place"));
    }

    #[test]
    fn trino_identifiers_refuse_controls_and_escape_double_quotes() {
        assert_eq!(trino_ident("erp_pg").unwrap(), "\"erp_pg\"");
        assert_eq!(trino_ident("a\"b").unwrap(), "\"a\"\"b\"");
        assert!(trino_ident("a\0b").is_err());
        assert!(trino_ident("").is_err());
    }

    #[test]
    fn clickhouse_identifiers_refuse_anything_but_plain_words() {
        assert_eq!(ch_ident("erp_pg").unwrap(), "`erp_pg`");
        assert!(ch_ident("a`b").is_err());
        assert!(ch_ident("a.b").is_err());
        assert!(ch_ident("").is_err());
    }

    #[tokio::test]
    async fn trino_introspection_filters_builtin_catalogs_and_schemas() {
        let server = MockServer::start().await;
        mount_trino(
            &server,
            "SHOW CATALOGS",
            trino_page(&["Catalog"], &[vec!["system"], vec!["jmx"], vec!["iceberg"], vec!["erp_pg"]]),
        )
        .await;
        mount_trino(
            &server,
            "SHOW SCHEMAS FROM \"erp_pg\"",
            trino_page(
                &["Schema"],
                &[vec!["information_schema"], vec!["pg_catalog"], vec!["public"]],
            ),
        )
        .await;
        mount_trino(
            &server,
            "SHOW TABLES FROM \"erp_pg\".\"public\"",
            trino_page(&["Table"], &[vec!["customers"], vec!["orders"]]),
        )
        .await;

        let tables = introspect_trino_tables(&trino_client(&server), 10).await.unwrap();
        assert_eq!(
            tables,
            vec![
                InPlaceTable {
                    catalog: Some("erp_pg".to_owned()),
                    schema: "public".to_owned(),
                    name: "customers".to_owned(),
                    full_name: "erp_pg.public.customers".to_owned(),
                    engine: "trino",
                },
                InPlaceTable {
                    catalog: Some("erp_pg".to_owned()),
                    schema: "public".to_owned(),
                    name: "orders".to_owned(),
                    full_name: "erp_pg.public.orders".to_owned(),
                    engine: "trino",
                },
            ]
        );
    }

    #[tokio::test]
    async fn trino_introspection_is_bounded_by_max_tables() {
        let server = MockServer::start().await;
        mount_trino(
            &server,
            "SHOW CATALOGS",
            trino_page(&["Catalog"], &[vec!["erp_pg"]]),
        )
        .await;
        mount_trino(
            &server,
            "SHOW SCHEMAS FROM \"erp_pg\"",
            trino_page(&["Schema"], &[vec!["public"]]),
        )
        .await;
        mount_trino(
            &server,
            "SHOW TABLES FROM \"erp_pg\".\"public\"",
            trino_page(&["Table"], &[vec!["a"], vec!["b"], vec!["c"]]),
        )
        .await;

        let tables = introspect_trino_tables(&trino_client(&server), 2).await.unwrap();
        assert_eq!(tables.len(), 2);
    }

    #[tokio::test]
    async fn trino_columns_are_read_from_describe() {
        let server = MockServer::start().await;
        mount_trino(
            &server,
            "DESCRIBE \"erp_pg\".\"public\".\"customers\"",
            trino_page(
                &["Column", "Type", "Extra", "Comment"],
                &[vec!["id", "bigint", "", ""], vec!["email", "varchar", "", ""]],
            ),
        )
        .await;

        let columns = introspect_trino_columns(&trino_client(&server), "erp_pg", "public", "customers")
            .await
            .unwrap();
        assert_eq!(
            columns,
            vec![
                InPlaceColumn { name: "id".to_owned(), data_type: "bigint".to_owned() },
                InPlaceColumn { name: "email".to_owned(), data_type: "varchar".to_owned() },
            ]
        );
    }

    #[tokio::test]
    async fn trino_upstream_error_text_never_reaches_the_error_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/statement"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "error": {"message": "SECRET_TABLE_NAME leaked by the coordinator", "errorCode": 1},
            })))
            .mount(&server)
            .await;

        let err = introspect_trino_tables(&trino_client(&server), 10)
            .await
            .expect_err("the engine refused");
        let message = err.to_string();
        assert!(!message.contains("SECRET_TABLE_NAME"), "{message}");
        assert!(message.contains("could not be inspected"), "{message}");
    }

    fn ch_page(data: Value) -> Value {
        json!({ "meta": [], "data": data, "rows": data.as_array().map_or(0, Vec::len) })
    }

    #[tokio::test]
    async fn clickhouse_introspection_lists_user_databases_only() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("FROM system.tables"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ch_page(json!([
                {"database": "system", "name": "hidden", "engine": "Memory"},
                {"database": "erp_pg", "name": "customers", "engine": "PostgreSQL"},
                {"database": "shop", "name": "orders", "engine": "MaterializedMySQL"},
            ]))))
            .mount(&server)
            .await;

        let tables = introspect_clickhouse_external_tables(&ch_client(&server), 10)
            .await
            .unwrap();
        assert_eq!(tables.len(), 2);
        assert_eq!(tables[0].full_name, "erp_pg.customers");
        assert!(!tables[0].full_name.starts_with("system."));
        assert_eq!(tables[1].full_name, "shop.orders");
        assert_eq!(tables[0].engine, "clickhouse");
    }

    #[tokio::test]
    async fn clickhouse_introspection_is_bounded_with_a_limit() {
        let server = MockServer::start().await;
        let received = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = received.clone();
        Mock::given(method("POST"))
            .and(body_string_contains("FROM system.tables"))
            .respond_with(move |req: &wiremock::Request| {
                seen.lock().unwrap().push(String::from_utf8_lossy(&req.body).into_owned());
                ResponseTemplate::new(200).set_body_json(ch_page(json!([])))
            })
            .mount(&server)
            .await;

        assert!(introspect_clickhouse_external_tables(&ch_client(&server), 5)
            .await
            .unwrap()
            .is_empty());
        let statements = received.lock().unwrap();
        assert_eq!(statements.len(), 1);
        assert!(statements[0].contains("LIMIT 5"));
    }

    #[tokio::test]
    async fn clickhouse_columns_are_read_from_system_columns() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("FROM system.columns"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ch_page(json!([
                {"name": "id", "type": "UInt64"},
                {"name": "email", "type": "String"},
            ]))))
            .mount(&server)
            .await;

        let columns = introspect_clickhouse_columns(&ch_client(&server), "erp_pg", "customers")
            .await
            .unwrap();
        assert_eq!(columns.len(), 2);
        assert_eq!(columns[1].data_type, "String");
    }

    #[tokio::test]
    async fn clickhouse_server_error_text_never_reaches_the_error_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500).set_body_string("SECRET_HOST:9000 refused"))
            .mount(&server)
            .await;

        let err = introspect_clickhouse_external_tables(&ch_client(&server), 5)
            .await
            .expect_err("the engine refused");
        let message = err.to_string();
        assert!(!message.contains("SECRET_HOST"), "{message}");
        assert!(message.contains("could not be inspected"), "{message}");
    }
}
