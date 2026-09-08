//! Gold export tools (T2.4 of the copilot-operations-handover plan):
//! `export_gold_mart`, `get_gold_export`.
//!
//! # Why these bypass `routes::gold`'s `check_export_token` guard
//!
//! `routes::gold::export`/`read_back` gate on
//! `check_export_token` — with `GOLD_EXPORT_RUN_TOKEN` unset (the common
//! case), that guard only lets a `PrincipalId::Service` principal through,
//! never a bare authenticated human session (see that module's doc
//! comment: "never a bare `RequiresAuth` pass, and never
//! unauthenticated"). A copilot tool call comes from a logged-in human's
//! chat session, so calling the HTTP handler as-is would make
//! `export_gold_mart` permanently unusable from chat on a deployment that
//! hasn't wired up a shared run token.
//!
//! This is the SAME shape `tools::alerts::run_alert_rule` already
//! establishes for `routes::alerts::run`/`check_run_token` (see that
//! module's doc comment): the run-token guard exists to restrict the HTTP
//! route to the Dagster scheduler's own service identity, which is the
//! wrong question for "a logged-in human, authorized by holding the
//! tool's own `spec.permission`, asks the copilot to export a mart now" —
//! the copilot's OWN gate (`super::super::gate::decide`) is the intended
//! authorization check for this call, not the cron door. So these tools
//! call [`crate::routes::gold::read_catalog_token`] and
//! [`crate::gold_export::{export_mart,read_back_row_count}`] directly —
//! the exact same functions `routes::gold::export`/`read_back` call AFTER
//! their token check passes — never a second copy of the Iceberg
//! read/write logic.

use axum::response::IntoResponse;
use lakehouse_core::ApiError;
use lakehouse_core::ident::Ident;
use serde_json::{Map, Value, json};

use super::{arg_str, response_to_value};
use crate::state::AppState;

/// Shared by both tools: validate `mart`, build the Iceberg client config
/// from the SAME Lakekeeper token file `routes::gold` reads. `Err` is
/// already a tool-shaped `Value` (via [`response_to_value`]) ready to
/// return as-is.
async fn iceberg_config_for(
    state: &AppState,
    mart: &str,
) -> Result<(Ident, lakehouse_iceberg::IcebergClientConfig), Value> {
    let mart_ident =
        Ident::new(mart).map_err(|e| json!({ "error": format!("mart tidak valid: {e}") }))?;
    let token =
        crate::routes::gold::read_catalog_token(&state.config.lakekeeper_gold_export_token_file)
            .await
            .map_err(|err| json!({ "error": err.to_string() }))?;
    let iceberg_config = crate::gold_export::iceberg_config(
        state.config.lakekeeper_catalog_uri.clone(),
        state.config.lakekeeper_warehouse.clone(),
        Some(token),
    );
    Ok((mart_ident, iceberg_config))
}

pub(super) async fn export_gold_mart(state: &AppState, args: &Map<String, Value>) -> Value {
    let mart = arg_str(args, "mart");
    if mart.is_empty() {
        return json!({ "error": "mart wajib diisi" });
    }
    let (mart_ident, iceberg_config) = match iceberg_config_for(state, &mart).await {
        Ok(pair) => pair,
        Err(err) => return err,
    };
    let source_table = format!(
        "{}.`{}`",
        state.config.gold_source_schema,
        mart_ident.as_str()
    );
    match crate::gold_export::export_mart(
        &state.clickhouse,
        &iceberg_config,
        &source_table,
        mart_ident.as_str(),
    )
    .await
    {
        Ok(result) => json!({
            "namespace": result.namespace,
            "table": result.table,
            "formatVersion": result.format_version,
            "rowsExported": result.rows_exported,
            "note": "Append-only: menjalankan ulang menambah baris baru, bukan menggantikan.",
        }),
        Err(err) => {
            response_to_value(crate::error::ApiRejection(ApiError::from(err)).into_response()).await
        }
    }
}

pub(super) async fn get_gold_export(state: &AppState, args: &Map<String, Value>) -> Value {
    let mart = arg_str(args, "mart");
    if mart.is_empty() {
        return json!({ "error": "mart wajib diisi" });
    }
    let (mart_ident, iceberg_config) = match iceberg_config_for(state, &mart).await {
        Ok(pair) => pair,
        Err(err) => return err,
    };
    match crate::gold_export::read_back_row_count(&iceberg_config, mart_ident.as_str()).await {
        Ok((format_version, rows)) => json!({
            "namespace": lakehouse_iceberg::gold::GOLD_NAMESPACE,
            "table": mart_ident.as_str(),
            "formatVersion": format_version,
            "rowsInIceberg": rows,
        }),
        Err(err) => {
            response_to_value(crate::error::ApiRejection(ApiError::from(err)).into_response()).await
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use super::*;
    use crate::config::Config;

    fn state() -> AppState {
        AppState::new(Config::from_map(&HashMap::new()).unwrap())
    }

    #[tokio::test]
    async fn both_tools_require_mart() {
        let s = state();
        assert_eq!(
            export_gold_mart(&s, &Map::new()).await,
            json!({ "error": "mart wajib diisi" })
        );
        assert_eq!(
            get_gold_export(&s, &Map::new()).await,
            json!({ "error": "mart wajib diisi" })
        );
    }

    /// With no Lakekeeper token file configured (the default test state),
    /// both tools fail cleanly with an error `Value` rather than panicking
    /// — proves the token-file read path is reached and its `ApiError` is
    /// surfaced as a tool result, not propagated.
    #[tokio::test]
    async fn both_tools_fail_cleanly_without_a_lakekeeper_token_file() {
        let s = state();
        let mut args = Map::new();
        args.insert("mart".to_owned(), json!("mart_wisman"));
        let export_result = export_gold_mart(&s, &args).await;
        assert!(export_result.get("error").is_some(), "{export_result}");
        let read_result = get_gold_export(&s, &args).await;
        assert!(read_result.get("error").is_some(), "{read_result}");
    }
}
