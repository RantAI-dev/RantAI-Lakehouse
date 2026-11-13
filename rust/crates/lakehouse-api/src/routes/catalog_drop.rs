//! Drop a table made by a file upload, and show whether it may be (`DATA-55`,
//! QA `LH-DATA-15-06`, `TC-1731`, decision `DEC-21`).
//!
//! QA's case needed a way to drop a Bronze table and then restore it from its
//! page. The restore already existed ([`super::catalog_restore`]); the drop did
//! not. These routes are the drop, on the table's own page:
//!
//! - `GET /api/catalog/{id}/drop` (`governance:write`) says whether this table
//!   may be dropped, and — only when asked with `?withRowCount=true` — how many
//!   rows it holds.
//! - `POST /api/catalog/{id}/drop` (`governance:write`) drops it, keeping the
//!   catalog's soft delete so the existing restore can bring it back for 7
//!   days.
//!
//! # What can be dropped (D4)
//!
//! Only a table a file upload made, and only for someone of the tenant whose
//! upload claimed the name. A connector's table, a pipeline's Silver or Gold
//! table, another tenant's upload's table and a name nobody claimed are all
//! refused with ONE sentence ([`ONLY_UPLOAD`]), so the answer never says whose
//! upload made a table. A table that is already dropped, one a load is running
//! into, and an install with no admin client ([`NOT_SET_UP`]) each have their
//! own refusal.
//!
//! # How the drop goes out (D2, D3, D8)
//!
//! As the catalog's admin principal — the identity that also undrops, so "who
//! drops is who undrops" — and never without the 7-day keep: the soft delete
//! profile is written before every drop, and if that write fails nothing is
//! dropped. Each failure before the catalog answers leaves nothing behind: the
//! audit entry `catalog.drop` with outcome `allowed` is written first (a drop
//! that cannot be audited does not happen), then the record, then the catalog
//! is asked; a failure removes the record again and writes `failed`.
//!
//! # What never happens
//!
//! Nothing the catalog says reaches a response. `OpenfgaError` carries no body
//! (AGENTS.md principle 4), and every sentence below is ours. No table is
//! purged, no registry row is deleted, and no file is touched (D12).

use std::sync::Arc;

use axum::Extension;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use lakehouse_auth::Principal;
use lakehouse_auth::openfga::{DELETE_RETENTION_SECONDS, LakekeeperAdminClient, OpenfgaError};
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::audit::NewAuditEvent;
use lakehouse_store::uploads::{self, NewTableDrop, TableClaim};
use serde::Deserialize;
use serde_json::{Value, json};
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use super::catalog::validate_annotation_id;
use super::catalog::{annotation_pool, catalog_tenant_refusal, ensure_asset_readable};
use super::catalog_profile::bronze_table_name;
use super::catalog_restore::BRONZE_NAMESPACE;
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::quality::count_rows;
use crate::state::AppState;

/// The refusal a table that is not an upload's gets — whichever of D4's first
/// three rules failed, so it never says whose upload made a table.
const ONLY_UPLOAD: &str = "Only a table made by a file upload can be dropped here.";
/// D4 rule 4: a drop inside its window is already on record (D9).
const ALREADY_DROPPED: &str = "This table is already dropped. It can be restored from its page.";
/// D4 rule 5: a load is running into the name.
const LOADING: &str = "A file is being loaded into this table, so it cannot be dropped yet.";
/// D4 rule 6: no admin client, so nothing could bring the table back.
const NOT_SET_UP: &str = "Tables cannot be dropped on this install: restore is not configured, so a dropped table could not come back.";
/// The 7-day keep could not be written; nothing was dropped.
const KEEP_UNCONFIRMED: &str = "The catalog could not confirm that a dropped table is kept for 7 days, so nothing was dropped. Try again shortly.";
/// The warehouse could not be looked up; nothing was dropped.
const UNREACHABLE: &str =
    "The catalog could not be reached, so nothing was dropped. Try again shortly.";
/// The `allowed` audit entry could not be written; nothing was dropped (D8).
const NOT_AUDITED: &str =
    "The drop could not be written to the audit trail, so nothing was dropped.";
/// The catalog holds no table of that name.
const NO_TABLE: &str = "There is no table to drop: the catalog holds no table of that name.";
/// The catalog refused the drop.
const REFUSED: &str = "The catalog refused to drop the table. Nothing was dropped.";
/// The catalog did not answer, so the outcome is not known — the page says which.
const UNKNOWN: &str = "The catalog did not answer, so it is not known whether the table was dropped. Open the table's page again: it says so if it was.";

/// D4, answered before anything is written: may this table be dropped, and by
/// whom.
enum Readiness {
    /// Every rule holds: the table, the tenant that claimed it, and the admin
    /// client a drop and its restore need.
    Ready {
        /// The raw table name, as the catalog knows it.
        table: String,
        /// The tenant whose upload claimed the name.
        tenant_id: Uuid,
        /// The catalog's admin client the drop goes out as (D3).
        admin: Arc<LakekeeperAdminClient>,
    },
    /// The first rule that failed, as the sentence to answer with.
    No(&'static str),
}

/// Apply D4 in order and stop at the first rule that fails, so a caller learns
/// the first reason a table cannot be dropped and nothing it may not know.
///
/// Rule 3 asks `any_connector_targets` unscoped on purpose: any connector's
/// target means the name is not only an upload's, and the one sentence says no
/// more than that.
///
/// Not [`super::catalog_restore::target`]: its "not configured" answer comes
/// before the questions D4 asks first, and it knows nothing of claims or loads.
///
/// # Errors
///
/// The `404 Asset not found` of an unknown slug ([`bronze_table_name`]), and
/// the store's and the pool's errors unchanged.
async fn assess(
    state: &AppState,
    pool: &PgPool,
    principal: &Principal,
    id: &str,
) -> Result<Readiness, ApiError> {
    if id.starts_with("silver.") || id.starts_with("serving.") {
        return Ok(Readiness::No(ONLY_UPLOAD));
    }
    let table = bronze_table_name(state, id).await?;
    if table.is_empty() {
        return Ok(Readiness::No(ONLY_UPLOAD));
    }
    let mut owner = None;
    for tenant_id in &principal.tenant_ids {
        if uploads::table_claim(pool, *tenant_id, &table).await? == TableClaim::Ours {
            owner = Some(*tenant_id);
            break;
        }
    }
    let Some(tenant_id) = owner else {
        return Ok(Readiness::No(ONLY_UPLOAD));
    };
    if lakehouse_store::connectors::any_connector_targets(pool, &table).await? {
        return Ok(Readiness::No(ONLY_UPLOAD));
    }
    if uploads::dropped_table(pool, &table).await?.is_some() {
        return Ok(Readiness::No(ALREADY_DROPPED));
    }
    for row in uploads::loading_into(pool, &table).await? {
        if super::uploads::settle_loading_upload(state, pool, row)
            .await
            .status
            == "ingesting"
        {
            return Ok(Readiness::No(LOADING));
        }
    }
    let Some(admin) = state.lakekeeper_admin.clone() else {
        return Ok(Readiness::No(NOT_SET_UP));
    };
    Ok(Readiness::Ready {
        table,
        tenant_id,
        admin,
    })
}

/// The `?withRowCount=` flag of the status read.
#[derive(Deserialize)]
pub struct DropQuery {
    /// `"true"` asks for the table's counted rows; anything else does not (the
    /// exact-string rule every boolean query value follows here).
    #[serde(default, rename = "withRowCount")]
    with_row_count: Option<String>,
}

/// `GET /api/catalog/{id}/drop` — may this table be dropped, and, when asked,
/// how many rows it holds. See the module doc.
///
/// The count is a scan of the whole table and costs up to 30 seconds, so it is
/// only paid when the dialog asks for it; opening the page costs no scan (D7).
///
/// # Errors
///
/// `400` for an over-long id; `403` with the tenant gate's reason; `404` when
/// no dataset has that slug or the caller's roles may not read it; `503` when
/// the store or the pool cannot be reached.
pub async fn status(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<DropQuery>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
) -> ApiResult<ApiJson<Value>> {
    if let Some(reason) = catalog_tenant_refusal(&state, &principal, &headers).await? {
        return Ok(ApiJson(json!({ "supported": false, "reason": reason })));
    }
    validate_annotation_id(&id)?;
    // GOV-7 D6: a hidden asset answers the same 404 as an unknown id.
    ensure_asset_readable(&state, &principal, &id).await?;
    let pool = annotation_pool(&state)?;
    Ok(ApiJson(
        match assess(&state, pool, &principal, &id).await? {
            Readiness::No(reason) => json!({ "droppable": false, "reason": reason }),
            Readiness::Ready { table, .. } => {
                let mut body = json!({
                    "droppable": true,
                    "table": format!("bronze.{table}"),
                    "keptDays": DELETE_RETENTION_SECONDS / 86_400,
                });
                if q.with_row_count.as_deref() == Some("true") {
                    // `None` is sent as `null` and shown as "could not be counted",
                    // never as 0 (D7).
                    let rows = count_rows(&state, &format!("bronze.{table}")).await;
                    body["rowCount"] = json!(rows);
                }
                body
            }
        },
    ))
}

/// The audit record of one drop, mirrored on `routes::uploads::
/// upload_audit_event`: `resource_kind` is `catalog`, paired with the asset's
/// id, and `args` names the table, its row count — `null` when it was not
/// counted — and the keep; never a row of the table (D8).
fn drop_event(
    principal: &Principal,
    id: &str,
    table: &str,
    rows: Option<i64>,
    outcome: &str,
    detail: Option<&str>,
) -> NewAuditEvent {
    NewAuditEvent {
        principal_id: Some(principal.id.uuid().to_string()),
        principal_kind: Some(principal.kind_for_audit().to_owned()),
        actor_label: Some(principal.display_name.clone()),
        action: "catalog.drop".to_owned(),
        resource_kind: Some("catalog".to_owned()),
        resource_id: Some(id.to_owned()),
        args: Some(json!({
            "table": table,
            "rowCount": rows,
            "keptDays": DELETE_RETENTION_SECONDS / 86_400,
        })),
        outcome: outcome.to_owned(),
        detail: detail.map(str::to_owned),
        run_id: None,
        approval_id: None,
        session_id: None,
    }
}

/// Write the `allowed` entry a drop (or a restore) must have before it happens
/// (D8). A failure is logged in full and refused with `refusal`: the action
/// does not happen unaudited, and the caller learns it in our own words.
///
/// # Errors
///
/// [`ApiError::Unavailable`] carrying `refusal` when the entry cannot be
/// written.
pub(super) async fn audit_first(
    pool: &PgPool,
    event: NewAuditEvent,
    refusal: &'static str,
) -> Result<(), ApiError> {
    let action = event.action.clone();
    if let Err(err) = lakehouse_store::audit::insert(pool, event).await {
        tracing::error!(?err, %action, "the audit entry of a catalog action could not be written");
        return Err(ApiError::Unavailable(refusal.to_owned()));
    }
    Ok(())
}

/// Write the outcome of a drop or a restore after the catalog answered (D8).
/// Best effort: the `allowed` entry already names who, which table and its row
/// count, so a failure here leaves the trail short one entry, not empty.
pub(super) async fn audit_after(pool: &PgPool, event: NewAuditEvent) {
    let action = event.action.clone();
    if let Err(err) = lakehouse_store::audit::insert(pool, event).await {
        tracing::warn!(?err, %action, "the outcome of a catalog action could not be audited");
    }
}

/// `POST /api/catalog/{id}/drop` — drops the table, keeping it restorable for
/// 7 days. See the module doc for the order and every refusal (D5, D8).
///
/// # Errors
///
/// `403` with the tenant gate's reason; `404` when no dataset has that slug or
/// the caller's roles may not read it; `409` when D4 refuses the table, when a
/// drop is already on record, or when the catalog has no such table or refused
/// the drop; `503` when the keep cannot be confirmed, the catalog cannot be
/// reached, the audit entry cannot be written, or the outcome is not known.
pub async fn drop_table(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
) -> ApiResult<ApiJson<Value>> {
    if let Some(reason) = catalog_tenant_refusal(&state, &principal, &headers).await? {
        return Err(ApiError::PermissionDenied(reason.to_owned()).into());
    }
    validate_annotation_id(&id)?;
    ensure_asset_readable(&state, &principal, &id).await?;
    let pool = annotation_pool(&state)?;
    let (table, tenant_id, admin) = match assess(&state, pool, &principal, &id).await? {
        Readiness::No(NOT_SET_UP) => {
            return Err(ApiError::Unavailable(NOT_SET_UP.to_owned()).into());
        }
        Readiness::No(reason) => return Err(ApiError::Conflict(reason.to_owned()).into()),
        Readiness::Ready {
            table,
            tenant_id,
            admin,
        } => (table, tenant_id, admin),
    };
    let Some(warehouse) = admin
        .find_warehouse_id(&state.config.lakekeeper_warehouse)
        .await
        .map_err(|err| {
            tracing::warn!(?err, "drop: the warehouse could not be looked up");
            ApiError::Unavailable(UNREACHABLE.to_owned())
        })?
    else {
        tracing::warn!(
            warehouse = %state.config.lakekeeper_warehouse,
            "drop: the catalog does not list the configured warehouse"
        );
        return Err(ApiError::Unavailable(UNREACHABLE.to_owned()).into());
    };
    // D3: never without the keep. Writing the same profile twice changes
    // nothing, and a warehouse whose init never ran (still on the catalog's
    // hard-delete default) is fixed here before anything can be dropped.
    admin
        .set_soft_delete_profile(&warehouse)
        .await
        .map_err(|err| {
            tracing::warn!(?err, "drop: the soft delete profile could not be written");
            ApiError::Unavailable(KEEP_UNCONFIRMED.to_owned())
        })?;
    // D7: the drop counts once more itself, for the record and the audit
    // entry; it never takes a number from the request.
    let rows = count_rows(&state, &format!("bronze.{table}")).await;
    audit_first(
        pool,
        drop_event(&principal, &id, &table, rows, "allowed", None),
        NOT_AUDITED,
    )
    .await?;
    // `DELETE_RETENTION_SECONDS` is 604,800; refuse rather than guess if the
    // constant ever stopped fitting, which fail-closed means refusing the drop
    // instead of writing a wrong window.
    let Ok(kept_seconds) = i64::try_from(DELETE_RETENTION_SECONDS) else {
        tracing::error!(
            seconds = DELETE_RETENTION_SECONDS,
            "drop: the retention does not fit in an i64"
        );
        return Err(ApiError::Internal("drop retention is out of range".to_owned()).into());
    };
    let record = uploads::record_table_drop(
        pool,
        &NewTableDrop {
            bronze_table: &table,
            tenant_id,
            asset_id: &id,
            dropped_by: &principal.display_name,
            row_count: rows,
            kept_seconds,
        },
    )
    .await?;
    let Some(record) = record else {
        audit_after(
            pool,
            drop_event(
                &principal,
                &id,
                &table,
                rows,
                "refused",
                Some("already dropped"),
            ),
        )
        .await;
        return Err(ApiError::Conflict(ALREADY_DROPPED.to_owned()).into());
    };
    match admin.drop_table(&warehouse, BRONZE_NAMESPACE, &table).await {
        Ok(true) => {
            audit_after(
                pool,
                drop_event(&principal, &id, &table, rows, "executed", None),
            )
            .await;
            Ok(ApiJson(drop_answer(&record, rows)?))
        }
        other => Err(
            catalog_did_not_drop(pool, &principal, &id, &table, rows, other)
                .await
                .into(),
        ),
    }
}

/// The answer of a drop the catalog made, from the record's two times (D8).
///
/// # Errors
///
/// [`ApiError::Internal`] when a timestamp cannot be written as RFC 3339,
/// which cannot happen for a time within seven days of now.
fn drop_answer(
    record: &lakehouse_store::uploads::TableDrop,
    rows: Option<i64>,
) -> Result<Value, ApiError> {
    let dropped_at = record
        .dropped_at
        .format(&Rfc3339)
        .map_err(|_err| ApiError::Internal("a timestamp could not be written".to_owned()))?;
    let expires_at = record
        .expires_at
        .format(&Rfc3339)
        .map_err(|_err| ApiError::Internal("a timestamp could not be written".to_owned()))?;
    Ok(json!({
        "ok": true,
        "table": format!("bronze.{}", record.bronze_table),
        "rowCount": rows,
        "droppedAt": dropped_at,
        "expiresAt": expires_at,
    }))
}

/// D8 step 5: the catalog did not drop the table, so the record is removed
/// again, the `failed` entry is written in our own words, and the caller gets
/// the sentence of D5. A failure to remove the record is logged and does not
/// change the answer: the record then closes the name until its window ends,
/// which is logged as the error it is.
async fn catalog_did_not_drop(
    pool: &PgPool,
    principal: &Principal,
    id: &str,
    table: &str,
    rows: Option<i64>,
    after: Result<bool, OpenfgaError>,
) -> ApiError {
    if let Err(err) = uploads::forget_table_drop(pool, table).await {
        tracing::error!(
            ?err,
            %table,
            "drop: the record of a drop the catalog did not make could not be removed"
        );
    }
    let (detail, error) = match after {
        Ok(false) => (
            "the catalog holds no such table",
            ApiError::Conflict(NO_TABLE.to_owned()),
        ),
        Err(OpenfgaError::Rejected) => (
            "the catalog refused the drop",
            ApiError::Conflict(REFUSED.to_owned()),
        ),
        _ => (
            "the catalog did not answer; the outcome is not known",
            ApiError::Unavailable(UNKNOWN.to_owned()),
        ),
    };
    audit_after(
        pool,
        drop_event(principal, id, table, rows, "failed", Some(detail)),
    )
    .await;
    error
}
