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
//!
//! # This route's boot posture: no hard dependency on Lakekeeper being provisioned
//!
//! `docker-compose.yml`'s `lakehouse-api` service does not (any more) wait
//! on `lakekeeper-authz-init` finishing before it starts — this crate's
//! whole documented posture (`lakehouse_store::connect_lazy`,
//! `AppState::pg`'s doc comment) is that the process boots and serves
//! every route it can even when a dependency is down, degrading only the
//! routes that need it. [`export`]/[`read_back`] report the missing
//! precondition at REQUEST time instead: [`read_catalog_token`] returns
//! `503 Unavailable` with fixed text naming the env var (ADR 0011, via the
//! shared `crate::lakekeeper_token::read_token_file` helper — never the
//! token path or the raw io error) when Gold export has not been
//! provisioned on this deployment yet, rather than the whole process
//! refusing to start over one route's dependency.
//!
//! # Single-flight: concurrent exports of the SAME mart
//!
//! [`export`] acquires a per-mart lock (`AppState::gold_export_locks` —
//! `crate::gold_lock`) before doing anything else; a second concurrent
//! call for the SAME mart gets `409 Conflict` immediately rather than
//! racing the first (both would read `ClickHouse` and append to Iceberg,
//! duplicating every row). See [`export`]'s own doc comment for the
//! design tradeoff (409, not queueing).

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
use crate::gold_export_history;
use crate::json::ApiJson;
use crate::state::AppState;

impl From<GoldExportError> for ApiError {
    fn from(err: GoldExportError) -> Self {
        match err {
            GoldExportError::ClickHouse(e) => Self::from(e),
            GoldExportError::UnsupportedColumn { .. }
            | GoldExportError::Batch(_)
            // The mart is over `GOLD_EXPORT_MAX_ROWS` — a well-formed
            // request against real (too-large) data, same "caller's input,
            // not our outage" shape as the other `Unprocessable` variants,
            // not a 500: the fix is retrying with a smaller mart or a
            // deliberately raised cap, not anything server-side.
            | GoldExportError::RowCapExceeded { .. } => Self::Unprocessable(err.to_string()),
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

/// See the module doc comment's "Auth" section. Two independent ways to
/// pass, checked in this order:
///
/// 1. **`gold:export` permission, unconditionally.** A principal (human or
///    service) holding this permission — e.g. Platform Admin's seeded
///    `*:*` role, or a narrower role an operator has explicitly granted it
///    — passes regardless of whether `GOLD_EXPORT_RUN_TOKEN` is
///    configured. This is what lets the console's "Export now" button
///    (WS6) work in every deployment, including ones that configure a
///    shared token for the Dagster schedule — D4 (`CHANGELOG.md`:
///    "`/api/alerts/run` now fails closed (401) when `ALERTS_RUN_TOKEN` is
///    unset, instead of allowing unauthenticated calls") forbids
///    fail-open when a token is unset; it does not require that a
///    configured token become the only accepted credential once one
///    exists — an explicitly permissioned session is exactly the kind of
///    caller this floor exists to let through, not turn away.
/// 2. **The configured shared token, if `GOLD_EXPORT_RUN_TOKEN` is set.**
///    Unchanged from before this permission fallback existed: a wrong or
///    missing token here is always `401`, whether or not the caller holds
///    any permission at all.
///
/// With neither set, only a `PrincipalId::Service` principal is let
/// through (`503` otherwise) — unchanged from before this task.
fn check_export_token(
    configured: Option<&str>,
    header_token: Option<&str>,
    query_token: Option<&str>,
    principal: Option<&Principal>,
) -> Result<(), ApiError> {
    if let Some(p) = principal
        && p.has("gold:export")
    {
        return Ok(());
    }
    if let Some(need) = configured {
        return if header_token.or(query_token) == Some(need) {
            Ok(())
        } else {
            Err(ApiError::unauthorized())
        };
    }
    match principal {
        // Unchanged: the scheduled Dagster trigger's own service identity
        // always passes even with no explicit scope, same posture as
        // before this task.
        Some(p) if matches!(p.id, PrincipalId::Service(_)) => Ok(()),
        _ => Err(ApiError::Unavailable(
            "gold export is not configured: set GOLD_EXPORT_RUN_TOKEN, call with a service \
             identity credential, or use a session with the gold:export permission"
                .to_owned(),
        )),
    }
}

/// Reads the `gold-export` principal's Lakekeeper bearer token from
/// [`crate::config::Config::lakekeeper_gold_export_token_file`].
///
/// Thin, Gold-specific wrapper over the shared
/// [`crate::lakekeeper_token::read_token_file`] helper (WS2 task A0
/// generalized this function rather than copying it for the new
/// `lakehouse-api-reader` principal — AGENTS.md rule 4). No token refresh:
/// the file is re-read on every call, so a re-minted token is picked up on
/// the next call, but nothing proactively re-mints one before its 30-day
/// expiry (ADR 0011's known gap).
///
/// # Errors
///
/// Returns [`ApiError::Unavailable`] with fixed text naming only the
/// purpose and `LAKEKEEPER_GOLD_EXPORT_TOKEN_FILE` if the file cannot be
/// read (Gold export is not provisioned on this deployment — see ADR
/// 0011: the `gold-export` principal's token is minted by `ops/oidc-mock`
/// at compose bring-up onto a volume this service must have mounted). The
/// path and the io error are logged, never returned to the caller.
///
/// `pub(crate)` (not private) so `routes::ai::tools::gold` (T2.4 of the
/// copilot-operations-handover plan) can read the SAME token this route
/// reads, rather than a second copy of this file-read — see that module's
/// doc comment for why the copilot tool calls this directly instead of
/// going through [`export`]/[`read_back`]'s [`check_export_token`] guard.
pub(crate) async fn read_catalog_token(path: &str) -> Result<SecretValue, ApiError> {
    crate::lakekeeper_token::read_token_file(
        path,
        "gold-export",
        "LAKEKEEPER_GOLD_EXPORT_TOKEN_FILE",
    )
    .await
}

/// `POST /api/gold/export/{mart}` — run the export.
///
/// # Single-flight: only one export of a given mart runs at a time
///
/// A per-mart lock (`AppState::gold_export_locks` — see
/// `crate::gold_lock`'s module doc comment) is acquired before anything
/// else happens. If a second call for the SAME mart arrives while the
/// first is still running, this returns `409 Conflict` immediately rather
/// than letting both calls read `ClickHouse` and append to Iceberg — two
/// concurrent exports of the same mart would each append the mart's
/// current rows, duplicating them in the target table (`GoldTable::append`
/// has no dedup). A DIFFERENT mart is unaffected — the lock is keyed by
/// mart name.
///
/// # Errors
///
/// Returns 401/503 from [`check_export_token`], `409` if an export of the
/// SAME mart is already in flight, 503 if the Lakekeeper token cannot be
/// read, 422 for an unsupported source column, a source mart over
/// `GOLD_EXPORT_MAX_ROWS`, or a bad `ClickHouse` query, or 500 for an
/// Iceberg catalog/write failure.
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
        Ident::new(&mart).map_err(|e| ApiError::BadRequest(format!("invalid mart: {e}")))?;

    // Held for the rest of this handler (dropped at function return,
    // success or error alike) — see this function's "Single-flight" doc
    // section above for why acquiring it is the very first thing done
    // after validating the mart name.
    let _export_guard = state
        .gold_export_locks
        .try_acquire(mart_ident.as_str())
        .await
        .map_err(|_| {
            ApiError::Conflict(format!(
                "export for mart {:?} is already running; wait for it to finish before retrying",
                mart_ident.as_str()
            ))
        })?;

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

    let started_at = time::OffsetDateTime::now_utc();
    let export_result = gold_export::export_mart(
        &state.clickhouse,
        &iceberg_config,
        &source_table,
        mart_ident.as_str(),
        state.config.gold_export_max_rows,
        state.config.gold_export_batch_size,
    )
    .await;
    let finished_at = time::OffsetDateTime::now_utc();

    let triggered_by = principal.as_ref().map_or_else(
        || "unknown".to_owned(),
        |Extension(p)| match p.id {
            PrincipalId::Service(_) => format!("service:{}", p.display_name),
            PrincipalId::User(_) => format!("user:{}", p.display_name),
        },
    );
    let error_message = export_result.as_ref().err().map(ToString::to_string);
    #[allow(
        clippy::cast_possible_truncation,
        reason = "rows_exported is a usize from an in-memory Vec of one ClickHouse batch, \
                  never near u64::MAX"
    )]
    let history_row = gold_export_history::NewGoldExportRun {
        mart: mart_ident.as_str(),
        status: if export_result.is_ok() {
            "success"
        } else {
            "failed"
        },
        rows_exported: export_result.as_ref().ok().map(|r| r.rows_exported as u64),
        format_version: export_result.as_ref().ok().map(|r| r.format_version),
        snapshot_id: export_result.as_ref().ok().and_then(|r| r.snapshot_id),
        error: error_message.as_deref(),
        triggered_by: &triggered_by,
        started_at_ms: started_at.unix_timestamp() * 1000,
        finished_at_ms: finished_at.unix_timestamp() * 1000,
    };
    // Best-effort: a history-write failure must never turn an otherwise
    // successful (or already-failed-for-a-different-reason) export into a
    // 500 — see gold_export_history::record_export_run's doc comment.
    if let Err(err) = gold_export_history::record_export_run(&state.clickhouse, &history_row).await
    {
        tracing::error!(%err, mart = mart_ident.as_str(), "failed to record gold export history");
    }

    let result = export_result.map_err(ApiError::from)?;

    Ok(ApiJson(json!({
        "namespace": result.namespace,
        "table": result.table,
        "formatVersion": result.format_version,
        "rowsExported": result.rows_exported,
        "snapshotId": result.snapshot_id,
        "exportedAt": result.exported_at_ms.and_then(millis_to_rfc3339),
    })))
}

/// Renders an `Iceberg` snapshot's Unix-millisecond commit time as an RFC
/// 3339 UTC string, or `None` if `ms` doesn't fall on a valid instant
/// (defensive only — every value this is ever called with comes from
/// `Snapshot::timestamp_ms`, which `iceberg-rust` derives from the
/// table's own committed metadata).
fn millis_to_rfc3339(ms: i64) -> Option<String> {
    time::OffsetDateTime::from_unix_timestamp(ms / 1000)
        .ok()
        .and_then(|dt| {
            dt.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
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
        Ident::new(&mart).map_err(|e| ApiError::BadRequest(format!("invalid mart: {e}")))?;

    let token = read_catalog_token(&state.config.lakekeeper_gold_export_token_file).await?;
    let iceberg_config = gold_export::iceberg_config(
        state.config.lakekeeper_catalog_uri.clone(),
        state.config.lakekeeper_warehouse.clone(),
        Some(token),
    );

    let readback = gold_export::read_back_row_count(&iceberg_config, mart_ident.as_str())
        .await
        .map_err(ApiError::from)?;

    Ok(ApiJson(json!({
        "namespace": lakehouse_iceberg::gold::GOLD_NAMESPACE,
        "table": mart_ident.as_str(),
        "formatVersion": readback.format_version,
        "rowsInIceberg": readback.rows,
        "snapshotId": readback.snapshot_id,
        "exportedAt": readback.exported_at_ms.and_then(millis_to_rfc3339),
    })))
}

/// Query parameters for `GET /api/gold/exports`.
#[derive(Debug, Deserialize)]
pub struct ExportsQuery {
    mart: String,
    /// Same shared-token/permission gate as `export`/`read_back` — see
    /// [`check_export_token`].
    token: Option<String>,
}

/// `GET /api/gold/exports?mart=` — the most recent export attempts for one
/// mart, newest first, from `console.gold_export_run`
/// (`gold_export_history`). Same auth posture as `read_back`: it reveals
/// row counts and timing, not public data.
///
/// # Errors
///
/// Returns 400 if `mart` is missing/blank or not a valid identifier,
/// 401/503 from [`check_export_token`], or 500 if the `ClickHouse` query
/// fails.
pub async fn exports(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ExportsQuery>,
    principal: Option<Extension<Principal>>,
) -> ApiResult<ApiJson<Value>> {
    let header_token = headers.get("x-run-token").and_then(|v| v.to_str().ok());
    check_export_token(
        state.config.gold_export_run_token.as_deref(),
        header_token,
        query.token.as_deref(),
        principal.as_ref().map(|Extension(p)| p),
    )?;

    let mart_ident = Ident::new(query.mart.trim())
        .map_err(|e| ApiError::BadRequest(format!("invalid mart: {e}")))?;

    let runs = gold_export_history::list_export_runs(&state.clickhouse, mart_ident.as_str(), 50)
        .await
        .map_err(ApiError::from)?;

    Ok(ApiJson(
        json!({ "mart": mart_ident.as_str(), "runs": runs }),
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use lakehouse_auth::PermissionSet;

    use super::*;

    fn service_principal() -> Principal {
        Principal {
            id: PrincipalId::Service(uuid::Uuid::new_v4()),
            tenant_ids: vec![],
            display_name: "gold-export-scheduler".to_owned(),
            permissions: PermissionSet::parse("gold:export"),
            provider: "service".to_owned(),
            must_change_password: false,
        }
    }

    fn human_principal_with(permissions: &str) -> Principal {
        Principal {
            id: PrincipalId::User(uuid::Uuid::new_v4()),
            tenant_ids: vec![],
            display_name: "operator".to_owned(),
            permissions: PermissionSet::parse(permissions),
            provider: "session".to_owned(),
            must_change_password: false,
        }
    }

    #[test]
    fn gold_export_permission_passes_even_with_no_token_configured() {
        assert!(
            check_export_token(None, None, None, Some(&human_principal_with("gold:export")))
                .is_ok()
        );
    }

    #[test]
    fn gold_export_permission_passes_even_when_a_token_is_configured_and_not_presented() {
        // D4 (CHANGELOG.md) forbids FAIL-OPEN — letting an unauthenticated
        // or under-permissioned caller through when a token is unset. It
        // does not require that a token, once configured, become the
        // ONLY accepted credential: a principal explicitly granted
        // gold:export (e.g. via a console session) is exactly the kind
        // of caller D4's floor already exists to distinguish from "any
        // signed-up user" — see check_export_token's doc comment. Without
        // this, a deployment that sets GOLD_EXPORT_RUN_TOKEN (for the
        // Dagster schedule) would make the console's "Export now" button
        // permanently unusable for every human operator.
        assert!(
            check_export_token(
                Some("secret"),
                None,
                None,
                Some(&human_principal_with("gold:export")),
            )
            .is_ok()
        );
    }

    #[test]
    fn no_configured_token_still_accepts_a_service_principal_with_no_explicit_scope() {
        // Unchanged behavior from before this task: a service principal
        // was always sufficient, regardless of its own permission set.
        let mut service = service_principal();
        service.permissions = PermissionSet::parse("");
        assert!(check_export_token(None, None, None, Some(&service)).is_ok());
    }

    #[test]
    fn no_configured_token_rejects_a_human_principal_without_the_permission() {
        let err = check_export_token(
            None,
            None,
            None,
            Some(&human_principal_with("catalog:read")),
        )
        .unwrap_err();
        assert!(matches!(err, ApiError::Unavailable(_)));
    }

    #[test]
    fn no_configured_token_rejects_no_principal_at_all() {
        assert!(check_export_token(None, None, None, None).is_err());
    }

    #[test]
    fn configured_token_rejects_an_unpermissioned_session_presenting_the_wrong_token() {
        // The token path's refusal is unchanged for anyone NOT holding
        // gold:export: a wrong/missing token still 401s exactly as
        // before this task.
        let err = check_export_token(
            Some("secret"),
            None,
            Some("wrong"),
            Some(&human_principal_with("catalog:read")),
        )
        .unwrap_err();
        assert!(
            matches!(err, ApiError::Unauthorized(_)) || err.to_string().contains("unauthorized")
        );
    }
}
