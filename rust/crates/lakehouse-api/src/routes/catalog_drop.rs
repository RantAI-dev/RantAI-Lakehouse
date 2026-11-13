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
//! - `GET /api/catalog/dropped/tables` (`catalog:read`) lists the caller's
//!   tenants' dropped tables while each can still be restored (D13).
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
//! dropped. The audit entry `catalog.drop` with outcome `allowed` is written
//! first (a drop that cannot be audited does not happen), then the record, then
//! the catalog is asked, and the closing `executed` or `failed` entry is written
//! last. The whole stretch runs in ONE task the handler awaits, so the route's
//! deadline or a caller that disconnects stops only the waiting (`DATA-55`
//! review R1).
//!
//! What a failure leaves behind depends on what is known (`DATA-55` review R2):
//! a drop the catalog definitely refused removes the record again; an outcome
//! nobody knows keeps it, so the name stays closed and a table that was dropped
//! is not mistaken for one that is absent; and a record that cannot be written
//! leaves the `allowed` entry and its `failed` ending, never an entry with no
//! ending (review R7). A record is believed while the table is absent, and a
//! record for a table that is there is healed only once it is too old for its
//! drop to still be in flight ([`dropped_now`], [`HEAL_GRACE`]), so a stale one
//! cannot close a live table's name and a fresh one cannot be lost.
//!
//! # What never happens
//!
//! Nothing the catalog says reaches a response. `OpenfgaError` carries no body
//! (AGENTS.md principle 4), and every sentence below is ours. No table is
//! purged, no registry row is deleted, and no file is touched (D12).

use std::sync::Arc;
use std::time::Duration;

use axum::Extension;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use lakehouse_auth::Principal;
use lakehouse_auth::openfga::{
    DELETE_RETENTION_SECONDS, DeletedTableLookup, LakekeeperAdminClient, OpenfgaError,
    is_plain_identifier,
};
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
/// The drop's own task did not finish, so the handler cannot claim an outcome
/// (`DATA-55` review R1).
const INTERRUPTED: &str = "The drop did not finish, so it is not known whether the table was dropped. Open the table's page again: it says so if it was.";

/// The grace a drop's record must have before "the table exists" may heal it
/// away (`DATA-55` review S1).
///
/// A drop writes its record first and sends the catalog's `DELETE` afterwards,
/// so from the insert until the catalog has applied the `DELETE` the table
/// exists and the record is TRUE. Healing therefore has to outlast every
/// stretch a drop can be in flight: the `DELETE`'s own fifteen seconds, the two
/// outcome checks' ten seconds each ([`OUTCOME_CHECK_TIMEOUT`]), and margin.
/// Two minutes covers all of it.
pub(super) const HEAL_GRACE: i64 = 120;

/// The bound on each of the two checks the unknown outcome makes (`DATA-55`
/// review R2): the deleted-tabulars list, and the table's own presence. Ten
/// seconds is a lookup, not a scan. Every catalog call inside the drop's or the
/// restore's spawned task is bounded by one of these or by the client's own
/// bound on its request (`DATA-55` review S2), so a hanging catalog cannot
/// leave an `allowed` entry with no ending.
pub(super) const OUTCOME_CHECK_TIMEOUT: Duration = Duration::from_secs(10);

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
/// Rule 1 also asks the client's own [`is_plain_identifier`] of the registry's
/// name (`DATA-55` review R7): a name with a `/`, a `.` or a percent-encoded
/// byte in it would address another catalog path, and the client would refuse it
/// only after the audit entry and the record were written.
///
/// Rule 3 asks `any_connector_targets` unscoped on purpose: any connector's
/// target means the name is not only an upload's, and the one sentence says no
/// more than that.
///
/// Rule 4 believes a record only while the table is absent ([`dropped_now`]).
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
    if table.is_empty() || !is_plain_identifier(&table) {
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
    if dropped_now(state, pool, &table).await? {
        return Ok(Readiness::No(ALREADY_DROPPED));
    }
    if loading_into_now(state, pool, &table).await? {
        return Ok(Readiness::No(LOADING));
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

/// Whether a load is `ingesting` into `table` right now: the row list settled
/// the way the upload routes settle it, so a load that already ended is not
/// counted as running (`DATA-55` D4 rule 5).
///
/// # Errors
///
/// The store's error when the loads cannot be read.
async fn loading_into_now(state: &AppState, pool: &PgPool, table: &str) -> Result<bool, ApiError> {
    for row in uploads::loading_into(pool, table).await? {
        if super::uploads::settle_loading_upload(state, pool, row)
            .await
            .status
            == "ingesting"
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether `table` is dropped right now, believing the record only while the
/// table is absent (`DATA-55` review R2, QA `LH-DATA-15-06`, `TC-1731`).
///
/// A kept record must not lock a live table for seven days, but it must not be
/// removed while the drop it records can still be in flight either
/// (`DATA-55` review S1). A table that IS there is healed only through
/// [`uploads::heal_table_drop`], which needs the record to be older than
/// [`HEAL_GRACE`] and unchanged since it was read; a younger record stands,
/// because the drop's `DELETE` may still be on its way and the table is there
/// until the catalog applies it. When the check itself fails, the record stands
/// too (fail closed): an answer that is not known is not an answer that the
/// name is free.
///
/// The one definition of the rule, used by the drop's own assessment
/// ([`assess`]) and by the load's [`super::uploads::ensure_table_free`] and
/// post-claim check (review R3), so the three cannot drift apart.
///
/// # Errors
///
/// The store's error when the record cannot be read. A failed healing removal
/// is logged and the record stands; it is not an error here.
pub(super) async fn dropped_now(
    state: &AppState,
    pool: &PgPool,
    table: &str,
) -> Result<bool, lakehouse_store::StoreError> {
    let Some(record) = uploads::dropped_table(pool, table).await? else {
        return Ok(false);
    };
    match super::catalog_restore::table_exists(state, table).await {
        Ok(true) => {
            match uploads::heal_table_drop(pool, table, record.dropped_at, HEAL_GRACE).await {
                Ok(true) => {
                    tracing::warn!(
                        %table,
                        "a drop was on record for a table that exists and the record was past its grace; the table was not dropped"
                    );
                    Ok(false)
                }
                // Younger than the grace, or replaced since the read: the drop
                // may still be in flight, so the record stands.
                Ok(false) => Ok(true),
                Err(err) => {
                    tracing::warn!(
                        ?err,
                        %table,
                        "a stale record of a drop could not be removed; the record stands"
                    );
                    Ok(true)
                }
            }
        }
        // Absent, or the check itself failed: the record stands.
        Ok(false) => Ok(true),
        Err(err) => {
            tracing::warn!(
                ?err,
                %table,
                "could not tell whether a recorded dropped table exists; the record stands"
            );
            Ok(true)
        }
    }
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
/// `400` for an over-long id; `404` when no dataset has that slug or the
/// caller's roles may not read it; `500` with the store's own `database error`
/// when the record cannot be read; `503` when no database pool is configured
/// ([`annotation_pool`]). A caller outside the tenant that owns the shared
/// catalog is NOT an error here: the answer is `200` with `supported: false`
/// and the gate's reason ([`super::catalog::catalog_tenant_refusal`]).
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
/// Everything from the first audit entry to the closing one runs in ONE
/// spawned task this handler awaits (`DATA-55` review R1): the route's own
/// deadline, or a client that disconnects, then stops only the waiting and can
/// never leave a record with no `executed` or `failed` behind it.
///
/// # Errors
///
/// `400` for an over-long id; `403` with the tenant gate's reason; `404` when
/// no dataset has that slug or the caller's roles may not read it; `409` when
/// D4 refuses the table, when a drop is already on record, when a load started
/// into it after the checks, or when the catalog has no such table or refused
/// the drop; `503` when dropping is not set up ([`NOT_SET_UP`]), the keep cannot
/// be confirmed, the catalog cannot be reached, the audit entry cannot be
/// written, or the outcome is not known; `500` when the store fails or the
/// drop's own task did not finish.
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
    let pool = annotation_pool(&state)?.clone();
    let (table, tenant_id, admin) = match assess(&state, &pool, &principal, &id).await? {
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
    // `DELETE_RETENTION_SECONDS` is 604,800; refuse rather than guess if the
    // constant ever stopped fitting, which fail-closed means refusing the drop
    // instead of writing a wrong window. Computed BEFORE the first audit entry
    // so an `allowed` entry always has an ending (`DATA-55` review R7).
    let Ok(kept_seconds) = i64::try_from(DELETE_RETENTION_SECONDS) else {
        tracing::error!(
            seconds = DELETE_RETENTION_SECONDS,
            "drop: the retention does not fit in an i64"
        );
        return Err(ApiError::Internal("drop retention is out of range".to_owned()).into());
    };
    let job = DropJob {
        state,
        pool,
        principal,
        id,
        table,
        tenant_id,
        admin,
        warehouse,
        rows,
        kept_seconds,
    };
    // `DATA-55` review R1: one task for the whole audited stretch, awaited here.
    // The checks above stay in the handler; what must not be cut in half by a
    // deadline or a disconnect is everything from `allowed` to its ending.
    let handle = tokio::spawn(async move { job.run().await });
    match handle.await {
        Ok(Ok(body)) => Ok(ApiJson(body)),
        Ok(Err(err)) => Err(err.into()),
        Err(join) => {
            tracing::error!(?join, "drop: the task that drops the table did not finish");
            Err(ApiError::Internal(INTERRUPTED.to_owned()).into())
        }
    }
}

/// The audited half of a drop: the `allowed` entry, the record, the catalog's
/// answer and the closing entry, in ONE spawned task (`DATA-55` review R1).
struct DropJob {
    /// The app state the load check settles through.
    state: AppState,
    /// The database the record and the audit trail live in.
    pool: PgPool,
    /// The person dropping the table, for both audit entries.
    principal: Principal,
    /// The asset id the entries and the answer name.
    id: String,
    /// The raw table name, as the catalog knows it.
    table: String,
    /// The tenant whose upload claimed the name.
    tenant_id: Uuid,
    /// The catalog's admin client the drop goes out as (D3).
    admin: Arc<LakekeeperAdminClient>,
    /// The catalog's warehouse id.
    warehouse: String,
    /// The whole table's row count, or `None` when it could not be counted.
    rows: Option<i64>,
    /// The catalog's keep, in seconds.
    kept_seconds: i64,
}

impl DropJob {
    /// Run the audited half of the drop and answer with the body of a drop the
    /// catalog made.
    ///
    /// Every catalog call in here has a bound of its own
    /// ([`OUTCOME_CHECK_TIMEOUT`], and the client's `DROP_TABLE_TIMEOUT` on the
    /// `DELETE`): the route's deadline stops only the handler's waiting, so a
    /// call without a bound would leave the `allowed` entry with no ending
    /// (`DATA-55` review S2).
    ///
    /// # Errors
    ///
    /// `409` and `503` of D5 (the catalog's refusal, a name already dropped, a
    /// load that started since the checks), the store's `500`, and the
    /// `500` of an answer that cannot be written as a body.
    async fn run(&self) -> Result<Value, ApiError> {
        audit_first(&self.pool, self.event("allowed", None), NOT_AUDITED).await?;
        let record = match uploads::record_table_drop(
            &self.pool,
            &NewTableDrop {
                bronze_table: &self.table,
                tenant_id: self.tenant_id,
                asset_id: &self.id,
                dropped_by: &self.principal.display_name,
                row_count: self.rows,
                kept_seconds: self.kept_seconds,
            },
        )
        .await
        {
            Ok(record) => record,
            Err(err) => {
                // `DATA-55` review R7: an `allowed` entry always has an ending.
                audit_after(
                    &self.pool,
                    self.event("failed", Some("the drop could not be recorded")),
                )
                .await;
                return Err(err.into());
            }
        };
        let Some(record) = record else {
            audit_after(&self.pool, self.event("refused", Some("already dropped"))).await;
            return Err(ApiError::Conflict(ALREADY_DROPPED.to_owned()));
        };
        // `DATA-55` review R3: the record is written first, and the load check
        // is asked a second time here, so a load that started between `assess`
        // and the record cannot pass the drop's check while the drop passes its
        // own.
        match loading_into_now(&self.state, &self.pool, &self.table).await {
            Ok(false) => {}
            Ok(true) => {
                if let Err(err) = uploads::forget_table_drop(&self.pool, &self.table).await {
                    tracing::error!(
                        ?err,
                        table = %self.table,
                        "drop: the record of a drop refused for a running load could not be removed"
                    );
                }
                audit_after(
                    &self.pool,
                    self.event("failed", Some("a load started into the table")),
                )
                .await;
                return Err(ApiError::Conflict(LOADING.to_owned()));
            }
            Err(err) => {
                tracing::error!(
                    ?err,
                    table = %self.table,
                    "drop: could not check the running loads once the record was written; the record stands"
                );
                audit_after(
                    &self.pool,
                    self.event(
                        "failed",
                        Some("the load into the table could not be checked"),
                    ),
                )
                .await;
                return Err(err);
            }
        }
        match self
            .admin
            .drop_table(&self.warehouse, BRONZE_NAMESPACE, &self.table)
            .await
        {
            Ok(true) => {
                audit_after(&self.pool, self.event("executed", None)).await;
                drop_answer(&record, self.rows)
            }
            other => self.settle_not_dropped(&record, other).await,
        }
    }

    /// The audit record of one drop, with this drop's own fields.
    fn event(&self, outcome: &str, detail: Option<&str>) -> NewAuditEvent {
        drop_event(
            &self.principal,
            &self.id,
            &self.table,
            self.rows,
            outcome,
            detail,
        )
    }

    /// The catalog did not (or may not) have dropped the table: what is known
    /// decides whether the record stays, and the sentence says which.
    ///
    /// `DATA-55` review R2, replacing D8 step 5: a definite refusal removes the
    /// record; an unknown outcome asks the catalog once, and keeps the record
    /// unless the catalog says the table is still there. The record is only
    /// removed when it is known that nothing was dropped — otherwise the name
    /// stays closed, because a dropped table that is not on record is exactly
    /// the load `D11` exists to refuse.
    async fn settle_not_dropped(
        &self,
        record: &lakehouse_store::uploads::TableDrop,
        after: Result<bool, OpenfgaError>,
    ) -> Result<Value, ApiError> {
        let (detail, error) = match after {
            Ok(false) => (
                "the catalog holds no such table",
                ApiError::Conflict(NO_TABLE.to_owned()),
            ),
            Err(OpenfgaError::Rejected) => (
                "the catalog refused the drop",
                ApiError::Conflict(REFUSED.to_owned()),
            ),
            // The catalog did not answer, or answered 5xx: the drop may have
            // happened. Ask once, with a bound, what the catalog lists.
            // (`Ok(true)` is answered in `run` before this; if it ever reached
            // here, the deleted list is what settles it.)
            Ok(true) | Err(OpenfgaError::Transport | OpenfgaError::Server) => {
                return self.settle_unknown_outcome(record).await;
            }
        };
        if let Err(err) = uploads::forget_table_drop(&self.pool, &self.table).await {
            tracing::error!(
                ?err,
                table = %self.table,
                "drop: the record of a drop the catalog did not make could not be removed"
            );
        }
        audit_after(&self.pool, self.event("failed", Some(detail))).await;
        Err(error)
    }

    /// Whether the drop happened, asked of the catalog once. The deleted list
    /// says a copy of the name was deleted, but that alone is not proof the
    /// drop went out (`DATA-55` review S3): an older deleted copy of the same
    /// name would make a failed `DELETE` read as a drop. Only a table that is
    /// GONE is a drop. Every other answer — listed but still there, not listed,
    /// too long to tell, or a check that failed or did not answer — keeps the
    /// record and answers D5's "not known" sentence (`DATA-55` review S1
    /// withdrew round 1's "not listed and the table is there" arm: a `DELETE`
    /// whose client gave up may still be applied by the catalog).
    ///
    /// Both catalog calls here are bounded ([`OUTCOME_CHECK_TIMEOUT`] and
    /// [`Self::bounded_table_exists`]): the task is no longer cut by the
    /// route's deadline, so an unbounded one would leave `allowed` with no
    /// ending.
    async fn settle_unknown_outcome(
        &self,
        record: &lakehouse_store::uploads::TableDrop,
    ) -> Result<Value, ApiError> {
        let lookup = tokio::time::timeout(
            OUTCOME_CHECK_TIMEOUT,
            self.admin
                .find_deleted_table(&self.warehouse, BRONZE_NAMESPACE, &self.table),
        )
        .await;
        let Ok(Ok(lookup)) = lookup else {
            return Err(self.keep_record_and_refuse().await);
        };
        if !matches!(lookup, DeletedTableLookup::Found(_)) {
            return Err(self.keep_record_and_refuse().await);
        }
        if self.bounded_table_exists().await != Some(false) {
            return Err(self.keep_record_and_refuse().await);
        }
        // The drop happened: the catalog keeps it, so the caller is told the
        // drop succeeded and the record stays.
        audit_after(&self.pool, self.event("executed", None)).await;
        drop_answer(record, self.rows)
    }

    /// Whether the table is there, asked inside the spawned task with a bound
    /// of its own (`DATA-55` review S2): the route's deadline no longer cuts
    /// this task, so an unbounded catalog call would leave the `allowed` entry
    /// with no ending. `None` is "not known" — the bound passed, or the lookup
    /// failed — and every caller treats that as the record standing.
    async fn bounded_table_exists(&self) -> Option<bool> {
        match tokio::time::timeout(
            OUTCOME_CHECK_TIMEOUT,
            super::catalog_restore::table_exists(&self.state, &self.table),
        )
        .await
        {
            Ok(Ok(exists)) => Some(exists),
            Ok(Err(err)) => {
                tracing::warn!(
                    ?err,
                    table = %self.table,
                    "drop: whether the table exists could not be checked; the record stands"
                );
                None
            }
            Err(_elapsed) => {
                tracing::warn!(
                    table = %self.table,
                    "drop: the check of the table's presence did not answer in time; the record stands"
                );
                None
            }
        }
    }

    /// Keep the record (fail closed: the name stays closed) and answer D5's
    /// "not known" sentence.
    async fn keep_record_and_refuse(&self) -> ApiError {
        audit_after(
            &self.pool,
            self.event("failed", Some("the outcome is not known")),
        )
        .await;
        ApiError::Unavailable(UNKNOWN.to_owned())
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

/// `GET /api/catalog/dropped/tables` — the caller's tenants' tables dropped
/// from the console, while each can still be restored (`DATA-55` D13).
///
/// The path has TWO literal segments so it can never shadow an asset's own
/// `/api/catalog/{id}`; `access-requests` is the same shape and the reason is
/// the same. The list is `catalog:read`, like the restore's status read: it
/// shows what can be restored, and restoring needs `governance:write`.
///
/// Rows the caller's roles may not read are dropped from the list, not
/// marked: the answer must not say which hidden tables exist (fail closed). An
/// unrestricted caller that belongs to no tenant is shown nothing, because the
/// list is read per tenant and "every tenant" is not, and never was, an answer
/// here (`list_dropped_tables`).
///
/// # Errors
///
/// `500` with the store's own `database error` when the list cannot be read;
/// `503` when no database pool is configured ([`annotation_pool`]). A caller
/// outside the tenant that owns the shared catalog is NOT an error here: the
/// answer is `200` with `supported: false`, the gate's reason and no tables.
pub async fn list(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
) -> ApiResult<ApiJson<Value>> {
    if let Some(reason) = catalog_tenant_refusal(&state, &principal, &headers).await? {
        return Ok(ApiJson(json!({
            "supported": false, "reason": reason, "tables": [],
        })));
    }
    let pool = annotation_pool(&state)?;
    let access = super::catalog::table_access_for(&state, &principal).await?;
    let rows = uploads::list_dropped_tables(pool, &principal.tenant_ids, 100).await?;
    let tables: Vec<Value> = rows
        .into_iter()
        .filter(|row| access.is_all() || access.allows_key(&format!("bronze.{}", row.bronze_table)))
        .filter_map(|row| {
            // A row whose time cannot be written is left out rather than shown
            // with a made-up date: the list exists to say how long is left.
            let dropped_at = row.dropped_at.format(&Rfc3339).ok()?;
            let expires_at = row.expires_at.format(&Rfc3339).ok()?;
            Some(json!({
                "assetId": row.asset_id,
                "table": format!("bronze.{}", row.bronze_table),
                "droppedBy": row.dropped_by,
                "rowCount": row.row_count,
                "droppedAt": dropped_at,
                "expiresAt": expires_at,
            }))
        })
        .collect();
    Ok(ApiJson(json!({ "tables": tables })))
}
