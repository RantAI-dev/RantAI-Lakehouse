//! Where a catalog asset's rows can be read from, for the routes that read
//! them on the caller's behalf: the detail sample (`catalog::detail`) and
//! the column profile (`catalog_profile`).
//!
//! A `silver.*`/`serving.*` id is its own `ClickHouse` table. A Bronze
//! registry slug is read from `silver.<table_name>` when that table exists
//! (what the detail sample always read), and otherwise from its Iceberg
//! table `bronze.<table_name>` through the `DataLakeCatalog` database
//! `Config::iceberg_query_db` names — the only path to an Iceberg-only
//! dataset.
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
/// does not list `DataLakeCatalog` tables. Any failure, including "no such
/// table" and an unreachable catalog, reads as "not available".
async fn iceberg_columns(ch: &ChClient, db: &Ident, table: &Ident) -> Vec<(String, String)> {
    let sql = format!("DESCRIBE TABLE {db}.`{BRONZE_NAMESPACE}.{table}`");
    ch.rows(&sql, None).await.map_or_else(
        |_| Vec::new(),
        |rows| {
            rows.iter()
                .map(|r| (str_col(r, "name").to_owned(), str_col(r, "type").to_owned()))
                .collect()
        },
    )
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

/// A Bronze dataset whose registry `table_name` is `table`: its Silver
/// table when there is one, else its Iceberg table when this deployment
/// has an Iceberg query database and the table exists there.
pub(crate) async fn bronze_source(
    state: &AppState,
    table: &str,
) -> Result<Option<ReadSource>, ChError> {
    if let Some(silver) = clickhouse_source(&state.clickhouse, "silver", table).await? {
        return Ok(Some(silver));
    }
    let (Some(db), Ok(table)) = (state.config.iceberg_query_db.as_ref(), Ident::new(table)) else {
        return Ok(None);
    };
    let columns = iceberg_columns(&state.clickhouse, db, &table).await;
    Ok((!columns.is_empty()).then(|| ReadSource {
        from: format!("{db}.`{BRONZE_NAMESPACE}.{table}`"),
        policy_key: format!("{BRONZE_NAMESPACE}.{table}"),
        kind: SourceKind::Iceberg,
        columns,
    }))
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

    /// A Silver table still wins, exactly as the detail sample always read.
    #[tokio::test]
    async fn a_silver_table_is_preferred() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("system.columns"))
            .respond_with(rows(&json!([{"name": "id", "type": "UInt64"}])))
            .mount(&server)
            .await;

        let source = bronze_source(&state(&server.uri(), Some("icecat_api")), "orders")
            .await
            .unwrap()
            .expect("resolved");

        assert_eq!(source.from, "silver.`orders`");
        assert_eq!(source.kind, SourceKind::ClickHouse);
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
}
