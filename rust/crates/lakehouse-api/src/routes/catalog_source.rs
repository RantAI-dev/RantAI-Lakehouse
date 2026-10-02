//! Where a catalog asset's rows can be read from, for the routes that read
//! them on the caller's behalf: the detail sample (`catalog::detail`) and
//! the column profile (`catalog_profile`).
//!
//! A `silver.*`/`serving.*` id is its own `ClickHouse` table. A Bronze
//! registry slug is read from its own Iceberg table `bronze.<table_name>`,
//! through the `DataLakeCatalog` database `Config::iceberg_query_db`
//! names. Only when that table cannot be read — the deployment has no
//! such database, or the registry row names no Iceberg table — is it read
//! from `silver.<table_name>`, the Silver model of the same name.
//!
//! It used to be the other way round: Silver whenever it existed. A
//! Bronze page then listed the Bronze table's columns beside statistics,
//! sample rows and a starter query of a different table — a Silver model
//! is deduplicated and keeps only the columns it conforms.
//!
//! Every name that reaches SQL here is an [`Ident`], and every
//! [`ReadSource`] carries the policy key the enforcement rewrite binds the
//! read to (`sql_rewrite::canonicalize` gives an Iceberg read the same key
//! as its Trino name, `bronze.<table>`), so callers look up masks against
//! exactly the table the rewrite will govern.

use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ident::{Ident, SqlLiteral};

use crate::routes::support::str_col;
use crate::state::AppState;

/// The Iceberg namespace every Bronze dataset lives in (dlt writes with
/// `dataset_name="bronze"`, see the module comment in `catalog.rs`).
const BRONZE_NAMESPACE: &str = "bronze";

/// Which engine-level thing a [`ReadSource`] reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceKind {
    /// A plain `ClickHouse` table.
    ClickHouse,
    /// An Iceberg table read through a `DataLakeCatalog` database.
    Iceberg,
}

/// A readable table behind a catalog asset, resolved and validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReadSource {
    /// The `FROM` target, ready to interpolate: ``silver.`orders` `` or
    /// ``icecat_api.`bronze.orders` ``.
    pub(crate) from: String,
    /// The key policies bind this table under (`silver.orders`,
    /// `bronze.orders`) — what `PolicyEngineObligations` is asked about.
    pub(crate) policy_key: String,
    pub(crate) kind: SourceKind,
    /// `(name, type)` in table order, as declared.
    pub(crate) columns: Vec<(String, String)>,
}

/// `(name, type)` of a `ClickHouse` table, in table order; empty when the
/// table does not exist.
async fn clickhouse_columns(
    ch: &ChClient,
    db: &Ident,
    table: &Ident,
) -> Result<Vec<(String, String)>, ChError> {
    let sql = format!(
        "SELECT name, type FROM system.columns WHERE database = {} AND table = {} ORDER BY position",
        SqlLiteral::from(db.as_str()),
        SqlLiteral::from(table.as_str())
    );
    Ok(ch
        .rows(&sql, None)
        .await?
        .iter()
        .map(|r| (str_col(r, "name").to_owned(), str_col(r, "type").to_owned()))
        .collect())
}

/// `(name, type)` of an Iceberg table via `DESCRIBE` — `system.columns`
/// does not list `DataLakeCatalog` tables. The error is kept: a caller that
/// has to tell "no such table" from "could not ask" needs it
/// ([`iceberg_table_presence`]).
async fn iceberg_describe(
    ch: &ChClient,
    db: &Ident,
    table: &Ident,
) -> Result<Vec<(String, String)>, ChError> {
    let sql = format!("DESCRIBE TABLE {db}.`{BRONZE_NAMESPACE}.{table}`");
    Ok(ch
        .rows(&sql, None)
        .await?
        .iter()
        .map(|r| (str_col(r, "name").to_owned(), str_col(r, "type").to_owned()))
        .collect())
}

/// [`iceberg_describe`] for a reader that only needs the columns. Any
/// failure, including "no such table" and an unreachable catalog, reads as
/// "not available".
async fn iceberg_columns(ch: &ChClient, db: &Ident, table: &Ident) -> Vec<(String, String)> {
    iceberg_describe(ch, db, table).await.unwrap_or_default()
}

/// A `ClickHouse` table `db.table`, when it exists.
pub(crate) async fn clickhouse_source(
    ch: &ChClient,
    db: &str,
    table: &str,
) -> Result<Option<ReadSource>, ChError> {
    let (Ok(db), Ok(table)) = (Ident::new(db), Ident::new(table)) else {
        return Ok(None);
    };
    let columns = clickhouse_columns(ch, &db, &table).await?;
    Ok((!columns.is_empty()).then(|| ReadSource {
        from: format!("{db}.`{table}`"),
        policy_key: format!("{db}.{table}"),
        kind: SourceKind::ClickHouse,
        columns,
    }))
}

/// A Bronze dataset whose registry `table_name` is `table`: its own
/// Iceberg table when this deployment can read it, else the Silver table
/// of the same name when there is one (see the module comment).
pub(crate) async fn bronze_source(
    state: &AppState,
    table: &str,
) -> Result<Option<ReadSource>, ChError> {
    if let Some(bronze) = iceberg_source(state, table).await {
        return Ok(Some(bronze));
    }
    clickhouse_source(&state.clickhouse, "silver", table).await
}

/// The Bronze Iceberg table `bronze.<table>` itself, when this deployment
/// has an Iceberg query database and the table exists there — never its
/// Silver counterpart. What a check written against `bronze.<table>`
/// reads.
pub(crate) async fn iceberg_source(state: &AppState, table: &str) -> Option<ReadSource> {
    let (Some(db), Ok(table)) = (state.config.iceberg_query_db.as_ref(), Ident::new(table)) else {
        return None;
    };
    let columns = iceberg_columns(&state.clickhouse, db, &table).await;
    (!columns.is_empty()).then(|| ReadSource {
        from: format!("{db}.`{BRONZE_NAMESPACE}.{table}`"),
        policy_key: format!("{BRONZE_NAMESPACE}.{table}"),
        kind: SourceKind::Iceberg,
        columns,
    })
}

/// Why [`iceberg_table_presence`] could not say whether a table exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PresenceUnknown {
    /// This deployment has no Iceberg query database (`ICEBERG_QUERY_DB`),
    /// so there is nothing to ask.
    NoQueryDatabase,
    /// `ClickHouse` or the catalog behind it did not answer, or answered
    /// with something other than "no such table". The detail is logged.
    Unanswered,
}

/// Whether the Bronze Iceberg table `bronze.<table>` exists, as a question
/// that can fail.
///
/// [`iceberg_source`] cannot serve a caller that must not assume a name is
/// free: it answers `None` for "no such table" and for "the catalog is
/// unreachable" alike. Here the first is `Ok(false)` (`ClickHouse`'s own
/// `UNKNOWN_TABLE`, the same test `routes::lakehouse` applies to a missing
/// results table) and everything else, including an unrecognised error, is
/// [`PresenceUnknown`], so a doubt is never read as absence.
///
/// # Errors
///
/// [`PresenceUnknown::NoQueryDatabase`] when no Iceberg query database is
/// configured; [`PresenceUnknown::Unanswered`] when the question got no
/// definite answer.
pub(crate) async fn iceberg_table_presence(
    state: &AppState,
    table: &str,
) -> Result<bool, PresenceUnknown> {
    let Some(db) = state.config.iceberg_query_db.as_ref() else {
        return Err(PresenceUnknown::NoQueryDatabase);
    };
    let Ok(table) = Ident::new(table) else {
        return Err(PresenceUnknown::Unanswered);
    };
    match iceberg_describe(&state.clickhouse, db, &table).await {
        Ok(columns) => Ok(!columns.is_empty()),
        Err(ChError::Server(body)) if super::lakehouse::is_unknown_table_error(&body) => Ok(false),
        Err(err) => {
            tracing::warn!(%err, "could not tell whether a Bronze Iceberg table exists");
            Err(PresenceUnknown::Unanswered)
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use serde_json::json;
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::config::Config;

    fn state(ch_url: &str, iceberg_db: Option<&str>) -> AppState {
        let mut env = HashMap::new();
        env.insert("CH_URL".to_owned(), ch_url.to_owned());
        if let Some(db) = iceberg_db {
            env.insert("ICEBERG_QUERY_DB".to_owned(), db.to_owned());
        }
        AppState::new(Config::from_map(&env).expect("a valid test Config"))
    }

    fn rows(data: &serde_json::Value) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({
            "meta": [{"name": "name", "type": "String"}, {"name": "type", "type": "String"}],
            "data": data,
            "rows": data.as_array().map_or(0, Vec::len),
        }))
    }

    /// No Silver table: an Iceberg-only dataset is read through the
    /// configured `DataLakeCatalog` database, under its Trino-equal key.
    #[tokio::test]
    async fn an_iceberg_only_dataset_resolves_through_the_query_database() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("system.columns"))
            .respond_with(rows(&json!([])))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains(
                "DESCRIBE TABLE icecat_api.`bronze.orders`",
            ))
            .respond_with(rows(&json!([{"name": "id", "type": "Int64"}])))
            .mount(&server)
            .await;

        let source = bronze_source(&state(&server.uri(), Some("icecat_api")), "orders")
            .await
            .unwrap()
            .expect("resolved");

        assert_eq!(source.from, "icecat_api.`bronze.orders`");
        assert_eq!(source.policy_key, "bronze.orders");
        assert_eq!(source.kind, SourceKind::Iceberg);
        assert_eq!(source.columns, vec![("id".to_owned(), "Int64".to_owned())]);
    }

    /// A Bronze dataset is read from its own table even when a Silver
    /// model of the same name exists: the model has other columns and
    /// other rows, and is an asset of its own.
    #[tokio::test]
    async fn the_bronze_table_is_read_even_when_a_silver_model_exists() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("system.columns"))
            .respond_with(rows(&json!([{"name": "id", "type": "UInt64"}])))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains(
                "DESCRIBE TABLE icecat_api.`bronze.orders`",
            ))
            .respond_with(rows(&json!([
                {"name": "id", "type": "Int64"},
                {"name": "note", "type": "Nullable(String)"},
            ])))
            .mount(&server)
            .await;

        let source = bronze_source(&state(&server.uri(), Some("icecat_api")), "orders")
            .await
            .unwrap()
            .expect("resolved");

        assert_eq!(source.from, "icecat_api.`bronze.orders`");
        assert_eq!(source.policy_key, "bronze.orders");
        assert_eq!(source.kind, SourceKind::Iceberg);
        assert_eq!(source.columns.len(), 2);
    }

    /// The Silver model is what is left when the Bronze table cannot be
    /// read: no such table in the query database, or no query database.
    #[tokio::test]
    async fn the_silver_model_is_the_fallback_when_bronze_cannot_be_read() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("system.columns"))
            .respond_with(rows(&json!([{"name": "id", "type": "UInt64"}])))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("DESCRIBE TABLE"))
            .respond_with(ResponseTemplate::new(404).set_body_string("UNKNOWN_TABLE"))
            .mount(&server)
            .await;

        for iceberg_db in [Some("icecat_api"), None] {
            let source = bronze_source(&state(&server.uri(), iceberg_db), "orders")
                .await
                .unwrap()
                .expect("resolved");
            assert_eq!(source.from, "silver.`orders`", "{iceberg_db:?}");
            assert_eq!(source.kind, SourceKind::ClickHouse);
        }
    }

    /// Without a configured query database there is no Iceberg path, and
    /// no DESCRIBE is ever sent.
    #[tokio::test]
    async fn no_query_database_means_no_iceberg_source() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("system.columns"))
            .respond_with(rows(&json!([])))
            .mount(&server)
            .await;

        let source = bronze_source(&state(&server.uri(), None), "orders")
            .await
            .unwrap();

        assert_eq!(source, None);
        let requests = server.received_requests().await.unwrap();
        assert!(
            requests
                .iter()
                .all(|r| !String::from_utf8_lossy(&r.body).contains("DESCRIBE"))
        );
    }

    /// A registry `table_name` that is not a plain identifier never
    /// reaches SQL.
    #[tokio::test]
    async fn an_unsafe_table_name_resolves_to_nothing() {
        let server = MockServer::start().await;
        let source = bronze_source(&state(&server.uri(), Some("icecat_api")), "x`; DROP")
            .await
            .unwrap();
        assert_eq!(source, None);
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    // ── iceberg_table_presence: a question that can fail ────────────────

    async fn presence(
        server: &MockServer,
        iceberg_db: Option<&str>,
    ) -> Result<bool, PresenceUnknown> {
        iceberg_table_presence(&state(&server.uri(), iceberg_db), "orders").await
    }

    /// The table is there: `DESCRIBE` returns its columns.
    #[tokio::test]
    async fn presence_is_true_when_describe_returns_columns() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains(
                "DESCRIBE TABLE icecat_api.`bronze.orders`",
            ))
            .respond_with(rows(&json!([{"name": "id", "type": "Int64"}])))
            .mount(&server)
            .await;

        assert_eq!(presence(&server, Some("icecat_api")).await, Ok(true));
    }

    /// `ClickHouse`'s own "no such table" is absence, and the only error that
    /// is. This is the body `ClickHouse` sends for a missing table.
    #[tokio::test]
    async fn presence_is_false_only_for_clickhouses_unknown_table_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("DESCRIBE TABLE"))
            .respond_with(ResponseTemplate::new(404).set_body_string(
                "Code: 60. DB::Exception: Table icecat_api.`bronze.orders` does not exist. \
                 (UNKNOWN_TABLE) (version 26.8.1.1 (official build))",
            ))
            .mount(&server)
            .await;

        assert_eq!(presence(&server, Some("icecat_api")).await, Ok(false));
    }

    /// Never "free" on a doubt: an error that is not "no such table" (the
    /// catalog is down, a setting is wrong, `ClickHouse` answers 500) and a
    /// `ClickHouse` that cannot be reached at all are both unknown, where
    /// `iceberg_source` would have said `None`.
    #[tokio::test]
    async fn presence_is_unknown_for_any_other_failure() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("DESCRIBE TABLE"))
            .respond_with(ResponseTemplate::new(500).set_body_string(
                "Code: 1000. DB::Exception: Poco::Exception. Connection refused (POCO_EXCEPTION)",
            ))
            .mount(&server)
            .await;
        assert_eq!(
            presence(&server, Some("icecat_api")).await,
            Err(PresenceUnknown::Unanswered)
        );
        // `iceberg_source` reads the same failure as an absent table.
        assert_eq!(
            iceberg_source(&state(&server.uri(), Some("icecat_api")), "orders").await,
            None
        );

        let nothing_listening = state("http://127.0.0.1:1", Some("icecat_api"));
        assert_eq!(
            iceberg_table_presence(&nothing_listening, "orders").await,
            Err(PresenceUnknown::Unanswered)
        );
    }

    /// No query database: nothing can be asked, and nothing is sent.
    #[tokio::test]
    async fn presence_is_unknown_without_a_query_database_and_sends_nothing() {
        let server = MockServer::start().await;
        assert_eq!(
            presence(&server, None).await,
            Err(PresenceUnknown::NoQueryDatabase)
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}
