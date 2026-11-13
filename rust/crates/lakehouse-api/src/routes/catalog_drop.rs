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

use axum::Extension;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use lakehouse_auth::Principal;
use lakehouse_auth::openfga::DELETE_RETENTION_SECONDS;
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::uploads::{self, TableClaim};
use serde::Deserialize;
use serde_json::{Value, json};

use super::catalog::validate_annotation_id;
use super::catalog::{annotation_pool, catalog_tenant_refusal, ensure_asset_readable};
use super::catalog_profile::bronze_table_name;
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

/// D4, answered before anything is written: may this table be dropped, and by
/// whom.
enum Readiness {
    /// Every rule holds: the table, the tenant that claimed it, and the admin
    /// client a drop and its restore need.
    Ready {
        /// The raw table name, as the catalog knows it.
        table: String,
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
    if owner.is_none() {
        return Ok(Readiness::No(ONLY_UPLOAD));
    }
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
    if state.lakekeeper_admin.is_none() {
        return Ok(Readiness::No(NOT_SET_UP));
    }
    Ok(Readiness::Ready { table })
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
