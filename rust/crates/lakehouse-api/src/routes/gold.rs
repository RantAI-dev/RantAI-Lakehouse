//! `POST`/`GET /api/gold/export/{mart}` — ADR 0010's Gold export to
//! Iceberg, triggered over HTTP.
//!
//! `POST` runs the export (read `{gold_source_schema}.{mart}` from
//! `ClickHouse`, append it to the Gold Iceberg table `{mart}` through
//! Lakekeeper — see `crate::gold_export`); `GET` reads the Gold Iceberg
//! table straight back through `iceberg-rust` and reports its row count
//! and format version, independent of whatever `POST` claimed — this is
//! the round-trip proof the acceptance test (`ops/gold_export/`) uses.
//!
//! This is a Rust/Phase-P6-only surface — no `TypeScript` route exists to
//! port, since the original backend never wrote Iceberg. The trigger this
//! build wires up is `dagster/dispar_orchestrate/gold_export.py`, the same
//! "Dagster calls the Rust API over HTTP" shape `routes::pipelines`
//! already uses in reverse (`lakehouse-api` calling Dagster) — here
//! Dagster is the caller.
//!
//! # Auth: same D4 shape as `/api/alerts/run`
//!
//! [`check_export_token`] mirrors `routes::alerts::check_run_token`
//! exactly: with `GOLD_EXPORT_RUN_TOKEN` configured, a matching
//! `x-run-token` header/`?token=` query param is required; with it unset,
//! only a `PrincipalId::Service` principal (the scheduler's own
//! credential) is let through — never a bare `RequiresAuth` pass, and
//! never unauthenticated. `POLICY_TABLE` still requires `RequiresAuth` as
//! the floor (see `crate::policy`), matching `/api/alerts/run`'s own
//! belt-and-suspenders shape.

use axum::extract::{Extension, Path, Query, State};
use axum::http::HeaderMap;
use lakehouse_auth::{Principal, PrincipalId};
use lakehouse_core::ApiError;
use lakehouse_core::ident::Ident;
use lakehouse_core::secret::SecretValue;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiResult;
use crate::gold_export::{self, GoldExportError};
use crate::json::ApiJson;
use crate::state::AppState;

impl From<GoldExportError> for ApiError {
    fn from(err: GoldExportError) -> Self {
        match err {
            GoldExportError::ClickHouse(e) => Self::from(e),
            GoldExportError::UnsupportedColumn { .. } | GoldExportError::Batch(_) => {
                Self::Unprocessable(err.to_string())
            }
            GoldExportError::Iceberg(e) => Self::Internal(e.to_string()),
        }
    }
}

/// Query parameters shared by `POST`/`GET /api/gold/export/{mart}`.
#[derive(Debug, Deserialize, Default)]
pub struct ExportQuery {
    /// Shared token, as a query-string fallback to the `x-run-token`
    /// header — same shape as `routes::alerts::RunQuery::token`.
    token: Option<String>,
}

/// See the module doc comment's "Auth" section; behaviorally identical to
/// `routes::alerts::check_run_token`, duplicated rather than shared
/// because the two guards protect different config fields and the
/// duplication is a handful of lines, not a maintained abstraction.
fn check_export_token(
    configured: Option<&str>,
    header_token: Option<&str>,
    query_token: Option<&str>,
    principal: Option<&Principal>,
) -> Result<(), ApiError> {
    if let Some(need) = configured {
        return if header_token.or(query_token) == Some(need) {
            Ok(())
        } else {
            Err(ApiError::unauthorized())
        };
    }
    match principal {
        Some(p) if matches!(p.id, PrincipalId::Service(_)) => Ok(()),
        _ => Err(ApiError::Unavailable(
            "gold export tidak dikonfigurasi: set GOLD_EXPORT_RUN_TOKEN, atau panggil dengan \
             kredensial service identity (bukan sesi pengguna manusia)"
                .to_owned(),
        )),
    }
}

/// Reads the `gold-export` principal's Lakekeeper bearer token from
/// [`crate::config::Config::lakekeeper_gold_export_token_file`].
///
/// # Errors
///
/// Returns [`ApiError::Unavailable`] if the file cannot be read (Gold
/// export is not provisioned on this deployment — see ADR 0011: the
/// `gold-export` principal's token is minted by `ops/oidc-mock` at compose
/// bring-up onto a volume this service must have mounted).
async fn read_catalog_token(path: &str) -> Result<SecretValue, ApiError> {
    let raw = tokio::fs::read_to_string(path).await.map_err(|err| {
        ApiError::Unavailable(format!(
            "Lakekeeper gold-export token tidak dapat dibaca dari {path:?}: {err} \
             (Gold export belum diprovisikan pada deployment ini — lihat ADR 0011)"
        ))
    })?;
    Ok(SecretValue::new(raw.trim().to_owned()))
}

/// `POST /api/gold/export/{mart}` — run the export.
///
/// # Errors
///
/// Returns 401/503 from [`check_export_token`], 503 if the Lakekeeper
/// token cannot be read, 422 for an unsupported source column or a bad
/// `ClickHouse` query, or 500 for an Iceberg catalog/write failure.
pub async fn export(
    State(state): State<AppState>,
    Path(mart): Path<String>,
    headers: HeaderMap,
    Query(query): Query<ExportQuery>,
    principal: Option<Extension<Principal>>,
) -> ApiResult<ApiJson<Value>> {
    let header_token = headers.get("x-run-token").and_then(|v| v.to_str().ok());
    check_export_token(
        state.config.gold_export_run_token.as_deref(),
        header_token,
        query.token.as_deref(),
        principal.as_ref().map(|Extension(p)| p),
    )?;

    let mart_ident =
        Ident::new(&mart).map_err(|e| ApiError::BadRequest(format!("mart tidak valid: {e}")))?;
    let source_table = format!(
        "{}.`{}`",
        state.config.gold_source_schema,
        mart_ident.as_str()
    );

    let token = read_catalog_token(&state.config.lakekeeper_gold_export_token_file).await?;
    let iceberg_config = gold_export::iceberg_config(
        state.config.lakekeeper_catalog_uri.clone(),
        state.config.lakekeeper_warehouse.clone(),
        Some(token),
    );

    let result = gold_export::export_mart(
        &state.clickhouse,
        &iceberg_config,
        &source_table,
        mart_ident.as_str(),
    )
    .await
    .map_err(ApiError::from)?;

    Ok(ApiJson(json!({
        "namespace": result.namespace,
        "table": result.table,
        "formatVersion": result.format_version,
        "rowsExported": result.rows_exported,
    })))
}

/// `GET /api/gold/export/{mart}` — read the Gold Iceberg table back
/// through `iceberg-rust` and report its row count, independent of
/// whatever the last `POST` claimed. This is the round-trip proof; it
/// does not touch `ClickHouse` at all.
///
/// # Errors
///
/// Returns 401/503 from [`check_export_token`] (same guard as `POST` —
/// this still reveals row counts, not public data), 503 if the Lakekeeper
/// token cannot be read, or 500 if the table does not exist yet or the
/// Iceberg read fails.
pub async fn read_back(
    State(state): State<AppState>,
    Path(mart): Path<String>,
    headers: HeaderMap,
    Query(query): Query<ExportQuery>,
    principal: Option<Extension<Principal>>,
) -> ApiResult<ApiJson<Value>> {
    let header_token = headers.get("x-run-token").and_then(|v| v.to_str().ok());
    check_export_token(
        state.config.gold_export_run_token.as_deref(),
        header_token,
        query.token.as_deref(),
        principal.as_ref().map(|Extension(p)| p),
    )?;

    let mart_ident =
        Ident::new(&mart).map_err(|e| ApiError::BadRequest(format!("mart tidak valid: {e}")))?;

    let token = read_catalog_token(&state.config.lakekeeper_gold_export_token_file).await?;
    let iceberg_config = gold_export::iceberg_config(
        state.config.lakekeeper_catalog_uri.clone(),
        state.config.lakekeeper_warehouse.clone(),
        Some(token),
    );

    let (format_version, rows) =
        gold_export::read_back_row_count(&iceberg_config, mart_ident.as_str())
            .await
            .map_err(ApiError::from)?;

    Ok(ApiJson(json!({
        "namespace": lakehouse_iceberg::gold::GOLD_NAMESPACE,
        "table": mart_ident.as_str(),
        "formatVersion": format_version,
        "rowsInIceberg": rows,
    })))
}
