//! Restore a dropped table from its asset page (`DATA-15` F4).
//!
//! A table dropped from the Iceberg catalog is kept by `Lakekeeper` for
//! seven days (the warehouse's soft delete profile, set by the compose
//! warehouse inits and by `LakekeeperAdminClient::ensure_warehouse`) and
//! can be undropped inside that window. These two routes show whether the
//! asset's table can be, and undrop it:
//!
//! - `GET /api/catalog/{id}/restore` (`catalog:read`) answers
//!   `{ supported: false, reason }` where restore does not apply or is not
//!   configured, `{ restorable: false, reason }` when there is nothing to
//!   restore, and `{ restorable: true, droppedAt, expiresAt }` when there is.
//! - `POST /api/catalog/{id}/restore` (`governance:write`) undrops it.
//!
//! Only a Bronze dataset's Iceberg table (namespace `bronze`) is covered:
//! Silver and Gold tables are `ClickHouse` tables, rewritten in place, with
//! no dropped version. A table dropped before soft delete shipped was
//! hard-deleted and is not recoverable; nothing here can say otherwise.
//!
//! The decision is [`assess`], a pure function of "does the table exist",
//! the dropped copy `Lakekeeper` lists and the clock, so every answer is
//! unit-tested without a server. Nothing `Lakekeeper` says reaches a
//! response: its failures are logged and answered with fixed sentences.
//!
//! # The audit trail, and the record (`DATA-55` D8, QA `LH-DATA-15-06`,
//! `TC-1731`)
//!
//! The drop route (`DATA-55`) makes this restore reachable from the console,
//! so the restore now follows the same order as the drop: `catalog.restore`
//! with outcome `allowed` is written BEFORE the undrop (a restore that cannot
//! be audited does not happen), then the catalog is asked, then `executed` or
//! `failed` is written. The record of the drop (`upload_table_drop`) is
//! cleared on both ends of a restore: when the undrop succeeds, and when the
//! table turns out to exist again anyway — but there only once the record is
//! past [`super::catalog_drop::HEAL_GRACE`], because until then the table may
//! be there simply because the drop's `DELETE` is still on its way. Either
//! clearing takes the record this restore READ before the undrop went out and
//! removes it only if that is still the record, so a second drop of the same
//! name written while the undrop is in flight keeps its own (`DATA-55` review
//! U2).

use std::sync::Arc;
use std::time::Duration;

use axum::Extension;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use iceberg::{NamespaceIdent, TableIdent};
use lakehouse_auth::Principal;
use lakehouse_auth::openfga::{DeletedTableLookup, LakekeeperAdminClient, OpenfgaError};
use lakehouse_core::ApiError;
use lakehouse_iceberg::rest::{self, RestError};
use lakehouse_store::PgPool;
use lakehouse_store::audit::NewAuditEvent;
use lakehouse_store::uploads;
use serde_json::{Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::catalog::{annotation_pool, catalog_tenant_refusal, validate_annotation_id};
use super::catalog_drop::{audit_after, audit_first};
use super::catalog_profile::bronze_table_name;
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::lakehouse_catalog::{self, CatalogAccessError};
use crate::state::AppState;

/// The namespace Bronze datasets' Iceberg tables live in.
pub(super) const BRONZE_NAMESPACE: &str = "bronze";

const NOT_CONFIGURED: &str = "restore is not configured on this install";
const NOT_ICEBERG: &str = "only an open table can be restored; this table is rewritten in place and keeps no dropped copy";
const NO_TABLE: &str = "this dataset has no table";
const EXISTS: &str = "the table exists";
const NOTHING: &str = "no dropped copy of this table is kept (a table dropped before restore was set up cannot come back)";
// PR #106 review R1 SHOULD-FIX DATA-15 2: answer "could not tell" when it stopped early
const COULD_NOT_TELL: &str =
    "could not tell whether a dropped copy of this table is kept (the deleted list is too long)";
const EXPIRED: &str = "the 7-day window to restore this table has passed";
const UNREACHABLE: &str = "the catalog could not be asked about this table; try again shortly";
/// The `allowed` audit entry could not be written; nothing was restored
/// (`DATA-55` D8, QA `LH-DATA-15-06`, `TC-1731`).
const NOT_AUDITED_RESTORE: &str =
    "The restore could not be written to the audit trail, so nothing was restored.";
/// The restore's own task did not finish (`DATA-55` review R1): the handler
/// cannot claim an outcome the task never reached.
const INTERRUPTED: &str = "The restore did not finish, so it is not known whether the table was restored. Open the table's page again: it says so if it was.";

/// The bound on the restore's own undrop (`DATA-55` review S2): the same
/// fifteen seconds the drop's own request has. The task is no longer cut by the
/// route's deadline, so an undrop that accepts the connection and never answers
/// would otherwise leave the `allowed` entry with no ending. An elapsed bound is
/// an unknown outcome, which the mirror case below settles.
const UNDROP_TIMEOUT: Duration = Duration::from_secs(15);

/// What can be done about the asset's table.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Assessment {
    /// A table of that name exists: nothing to restore, and an undrop
    /// would collide with it.
    Exists,
    /// `Lakekeeper` lists no dropped copy.
    Nothing,
    /// `Lakekeeper` deleted-tabulars list reached page limit while more pages remained.
    CouldNotTell,
    /// A dropped copy is listed, but its window has passed.
    Expired {
        dropped_at: String,
        expires_at: String,
    },
    /// A dropped copy can be undropped until `expires_at`.
    Restorable {
        id: String,
        dropped_at: String,
        expires_at: String,
    },
}

/// The one decision, from facts. `Err` only when the listed expiry cannot
/// be read, which is neither "restorable" nor "expired".
///
/// # Errors
///
/// [`ApiError::Unavailable`] when the dropped copy's expiry is not RFC 3339.
// PR #106 review R1 SHOULD-FIX DATA-15 2
pub(crate) fn assess(
    exists: bool,
    dropped: DeletedTableLookup,
    now: OffsetDateTime,
) -> Result<Assessment, ApiError> {
    if exists {
        return Ok(Assessment::Exists);
    }
    match dropped {
        DeletedTableLookup::None => Ok(Assessment::Nothing),
        DeletedTableLookup::CouldNotTell => Ok(Assessment::CouldNotTell),
        DeletedTableLookup::Found(dropped) => {
            let expires = OffsetDateTime::parse(&dropped.expires_at, &Rfc3339)
                .map_err(|_| ApiError::Unavailable(UNREACHABLE.to_owned()))?;
            if expires <= now {
                Ok(Assessment::Expired {
                    dropped_at: dropped.deleted_at,
                    expires_at: dropped.expires_at,
                })
            } else {
                Ok(Assessment::Restorable {
                    id: dropped.id,
                    dropped_at: dropped.deleted_at,
                    expires_at: dropped.expires_at,
                })
            }
        }
    }
}

/// Where an asset's restore stands before `Lakekeeper` is asked.
enum Target {
    Unsupported(&'static str),
    Ready {
        admin: Arc<LakekeeperAdminClient>,
        table: String,
    },
}

async fn target(state: &AppState, id: &str) -> Result<Target, ApiError> {
    if id.starts_with("silver.") || id.starts_with("serving.") {
        return Ok(Target::Unsupported(NOT_ICEBERG));
    }
    let table = bronze_table_name(state, id).await?;
    if table.is_empty() {
        return Ok(Target::Unsupported(NO_TABLE));
    }
    match state.lakekeeper_admin.clone() {
        Some(admin) => Ok(Target::Ready { admin, table }),
        None => Ok(Target::Unsupported(NOT_CONFIGURED)),
    }
}

/// A `Lakekeeper` management failure, logged by class and answered with a
/// fixed sentence (principle 4).
fn admin_unavailable(err: &OpenfgaError) -> ApiError {
    tracing::warn!(?err, "restore: the Lakekeeper management API failed");
    ApiError::Unavailable(UNREACHABLE.to_owned())
}

/// Whether the Iceberg table exists now, through the reader catalog every
/// other asset-page read uses.
///
/// `pub(super)` because the drop's healing rule (`DATA-55` review R2,
/// [`super::catalog_drop::dropped_now`]) asks it too: a record is believed
/// whether or not the table is there, and a table that exists only heals it
/// once the record is past its grace. This is the one way this service asks
/// "is the table there".
///
/// # Errors
///
/// The reader token's own `503`, and [`ApiError::Unavailable`] when the
/// catalog cannot answer.
pub(super) async fn table_exists(state: &AppState, table: &str) -> Result<bool, ApiError> {
    let namespace = NamespaceIdent::from_strs([BRONZE_NAMESPACE])
        .map_err(|_| ApiError::Internal("invalid namespace".to_owned()))?;
    let ident = TableIdent::new(namespace, table.to_owned());
    let found = lakehouse_catalog::call(state, move |client| {
        let ident = ident.clone();
        async move { rest::load_table_summary(&client, &ident).await }
    })
    .await;
    match found {
        Ok(_) => Ok(true),
        Err(CatalogAccessError::Rest(RestError::NotFound)) => Ok(false),
        Err(CatalogAccessError::Token(api_err)) => Err(api_err),
        Err(CatalogAccessError::Rest(rest_err)) => {
            tracing::warn!(%rest_err, "restore: the table could not be looked up");
            Err(ApiError::Unavailable(UNREACHABLE.to_owned()))
        }
    }
}

/// The assessment and the ids an undrop needs: `(assessment, warehouse id)`.
async fn inspect(
    state: &AppState,
    admin: &LakekeeperAdminClient,
    table: &str,
) -> Result<(Assessment, Option<String>), ApiError> {
    if table_exists(state, table).await? {
        return Ok((Assessment::Exists, None));
    }
    let warehouse = admin
        .find_warehouse_id(&state.config.lakekeeper_warehouse)
        .await
        .map_err(|err| admin_unavailable(&err))?
        .ok_or_else(|| ApiError::Unavailable(UNREACHABLE.to_owned()))?;
    let dropped = admin
        .find_deleted_table(&warehouse, BRONZE_NAMESPACE, table)
        .await
        .map_err(|err| admin_unavailable(&err))?;
    Ok((
        assess(false, dropped, OffsetDateTime::now_utc())?,
        Some(warehouse),
    ))
}

/// Clear a leftover drop record for a table that exists again — only through
/// the guarded removal (`DATA-55` review S1, [`uploads::heal_table_drop`]): the
/// record must be older than [`super::catalog_drop::HEAL_GRACE`] and unchanged
/// since it was read, because a drop that was just recorded may still have its
/// `DELETE` on the way and the table is there until the catalog applies it.
/// Best effort: a failure is logged and the caller's `409` is the same either
/// way.
async fn clear_stale_record(pool: &PgPool, table: &str) {
    match uploads::dropped_table(pool, table).await {
        Ok(Some(record)) => {
            match uploads::heal_table_drop(
                pool,
                table,
                record.dropped_at,
                super::catalog_drop::HEAL_GRACE,
            )
            .await
            {
                Ok(true) => tracing::warn!(
                    %table,
                    "restore: a drop record past its grace was cleared for a table that exists"
                ),
                Ok(false) => {}
                Err(err) => tracing::warn!(
                    ?err,
                    %table,
                    "restore: a leftover drop record could not be cleared"
                ),
            }
        }
        Ok(None) => {}
        Err(err) => tracing::warn!(
            ?err,
            %table,
            "restore: a leftover drop record could not be read"
        ),
    }
}

/// The audit record of one restore, mirrored on `routes::uploads::
/// upload_audit_event`: `resource_kind` is `catalog`, paired with the asset's
/// id, and `args` names the table and until when it had been restorable
/// (`DATA-15` F4). `principal_kind` comes from [`Principal::kind_for_audit`],
/// never a literal `"user"` (a service identity restores too).
fn restore_event(
    principal: &Principal,
    id: &str,
    table: &str,
    was_restorable_until: &str,
    outcome: &str,
    detail: Option<&str>,
) -> NewAuditEvent {
    NewAuditEvent {
        principal_id: Some(principal.id.uuid().to_string()),
        principal_kind: Some(principal.kind_for_audit().to_owned()),
        actor_label: Some(principal.display_name.clone()),
        action: "catalog.restore".to_owned(),
        resource_kind: Some("catalog".to_owned()),
        resource_id: Some(id.to_owned()),
        args: Some(json!({ "table": table, "wasRestorableUntil": was_restorable_until })),
        outcome: outcome.to_owned(),
        detail: detail.map(str::to_owned),
        run_id: None,
        approval_id: None,
        session_id: None,
    }
}

/// `GET /api/catalog/{id}/restore` — see the module doc.
///
/// # Errors
///
/// `400` for an over-long id; `404` when no dataset has that slug; `503`
/// with a fixed sentence when the catalog or `Lakekeeper` cannot be asked.
pub async fn status(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
) -> ApiResult<ApiJson<Value>> {
    if let Some(reason) = catalog_tenant_refusal(&state, &principal, &headers).await? {
        return Ok(ApiJson(json!({ "supported": false, "reason": reason })));
    }
    validate_annotation_id(&id)?;
    // GOV-7 D6: a hidden asset's restore status answers the same 404 as an
    // unknown id.
    crate::routes::catalog::ensure_asset_readable(&state, &principal, &id).await?;
    let (admin, table) = match target(&state, &id).await? {
        Target::Ready { admin, table } => (admin, table),
        Target::Unsupported(reason) => {
            return Ok(ApiJson(json!({ "supported": false, "reason": reason })));
        }
    };
    let (assessment, _) = inspect(&state, &admin, &table).await?;
    Ok(ApiJson(match assessment {
        Assessment::Exists => json!({ "restorable": false, "reason": EXISTS }),
        Assessment::Nothing => json!({ "restorable": false, "reason": NOTHING }),
        Assessment::CouldNotTell => json!({ "restorable": false, "reason": COULD_NOT_TELL }),
        Assessment::Expired {
            dropped_at,
            expires_at,
        } => json!({
            "restorable": false, "reason": EXPIRED,
            "droppedAt": dropped_at, "expiresAt": expires_at,
        }),
        Assessment::Restorable {
            dropped_at,
            expires_at,
            ..
        } => json!({
            "restorable": true, "droppedAt": dropped_at, "expiresAt": expires_at,
        }),
    }))
}

/// `POST /api/catalog/{id}/restore` — undrops the table.
///
/// Everything from the first audit entry to the closing one runs in ONE
/// spawned task this handler awaits (`DATA-55` review R1), for the reason the
/// drop does: a deadline or a disconnect must not stop the work between the
/// entry and its ending.
///
/// # Errors
///
/// `403` with the tenant gate's reason; `404` when the asset is unknown, when
/// the caller's roles may not read it, or when nothing is restorable (or the
/// asset has no restorable kind of table); `409` when a table of that name
/// exists again; `410` when the 7-day window has passed; `500` when the
/// restore's own task did not finish; `503` when restore is not configured, the
/// audit entry cannot be written, or `Lakekeeper` cannot be asked.
pub async fn restore(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
) -> ApiResult<ApiJson<Value>> {
    if let Some(reason) = catalog_tenant_refusal(&state, &principal, &headers).await? {
        return Err(ApiError::PermissionDenied(reason.to_owned()).into());
    }
    validate_annotation_id(&id)?;
    // GOV-7 D6: a hidden asset's restore answers the same 404 as an unknown
    // id, exactly as the status read above does.
    crate::routes::catalog::ensure_asset_readable(&state, &principal, &id).await?;
    let pool = annotation_pool(&state)?.clone();
    let (admin, table) = match target(&state, &id).await? {
        Target::Ready { admin, table } => (admin, table),
        Target::Unsupported(reason) if reason == NOT_CONFIGURED => {
            return Err(ApiError::Unavailable(NOT_CONFIGURED.to_owned()).into());
        }
        Target::Unsupported(_) => {
            return Err(ApiError::NotFound(
                "this asset has no dropped table to restore".to_owned(),
            )
            .into());
        }
    };
    let (assessment, warehouse) = inspect(&state, &admin, &table).await?;
    let (tabular, expires_at) = match assessment {
        Assessment::Exists => {
            // `DATA-55` D8 (QA `LH-DATA-15-06`, `TC-1731`): a table that
            // is there is not dropped, whatever a leftover record says — and
            // only a record past its grace is cleared here (`DATA-55` review
            // S1): the drop it records may still be in flight, and the table is
            // there until the catalog applies the `DELETE`.
            clear_stale_record(&pool, &table).await;
            return Err(ApiError::Conflict(
                "a table of that name exists again; it was recreated by a later load".to_owned(),
            )
            .into());
        }
        Assessment::Nothing => {
            return Err(ApiError::NotFound(
                "there is no dropped copy of this table to restore".to_owned(),
            )
            .into());
        }
        Assessment::CouldNotTell => {
            return Err(ApiError::Unavailable(COULD_NOT_TELL.to_owned()).into());
        }
        Assessment::Expired { .. } => return Err(ApiError::Gone(EXPIRED.to_owned()).into()),
        Assessment::Restorable { id, expires_at, .. } => (id, expires_at),
    };
    let Some(warehouse) = warehouse else {
        return Err(ApiError::Unavailable(UNREACHABLE.to_owned()).into());
    };
    // `DATA-55` review U2: the record of the drop this restore may clear, read
    // BEFORE the undrop goes out, so the clearing removes that record and never
    // one a second drop of the same name writes while the undrop is in flight
    // (the old record can be past its grace, so a read heals it and a new one is
    // written). No record, or an unreadable one: nothing is cleared, and the
    // next read heals a leftover.
    let dropped_at = match uploads::dropped_table(&pool, &table).await {
        Ok(Some(record)) => Some(record.dropped_at),
        Ok(None) => None,
        Err(err) => {
            tracing::warn!(
                ?err,
                %table,
                "restore: the record of the drop could not be read; this restore clears no record"
            );
            None
        }
    };
    let job = RestoreJob {
        state,
        pool,
        principal,
        id,
        table,
        admin,
        warehouse,
        tabular,
        expires_at,
        dropped_at,
    };
    // `DATA-55` review R1: one task for the audited stretch, awaited here.
    let handle = tokio::spawn(async move { job.run().await });
    match handle.await {
        Ok(Ok(body)) => Ok(ApiJson(body)),
        Ok(Err(err)) => Err(err.into()),
        Err(join) => {
            tracing::error!(
                ?join,
                "restore: the task that restores the table did not finish"
            );
            Err(ApiError::Internal(INTERRUPTED.to_owned()).into())
        }
    }
}

/// The audited half of a restore: the `allowed` entry, the undrop, the record's
/// end and the closing entry, in ONE spawned task (`DATA-55` review R1).
struct RestoreJob {
    /// The app state the table's presence is checked through.
    state: AppState,
    /// The database the record and the audit trail live in.
    pool: PgPool,
    /// The person restoring, for both audit entries.
    principal: Principal,
    /// The asset id the entries name.
    id: String,
    /// The raw table name.
    table: String,
    /// The catalog's admin client the undrop goes out as.
    admin: Arc<LakekeeperAdminClient>,
    /// The catalog's warehouse id.
    warehouse: String,
    /// The soft-deleted table's id, which the undrop takes.
    tabular: String,
    /// Until when it was restorable, as the audit entry records it.
    expires_at: String,
    /// The `dropped_at` of the record read before the undrop, which is the ONLY
    /// record this restore may clear (`DATA-55` review U2); `None` when there was
    /// no record to read, and then nothing is cleared.
    dropped_at: Option<OffsetDateTime>,
}

impl RestoreJob {
    /// Undrop the table and answer `{ "ok": true }`.
    ///
    /// Every catalog call in here has a bound of its own ([`UNDROP_TIMEOUT`]
    /// here, [`super::catalog_drop::OUTCOME_CHECK_TIMEOUT`] in the mirror
    /// case): the route's deadline stops only the handler's waiting, so a call
    /// without a bound would leave the `allowed` entry with no ending
    /// (`DATA-55` review S2).
    ///
    /// # Errors
    ///
    /// `409` when the catalog refuses the undrop, `503` when it cannot be asked
    /// or when the `allowed` entry cannot be written. The record's clearing is
    /// best effort: a failure is logged, never an error of this function.
    async fn run(&self) -> Result<Value, ApiError> {
        // `DATA-55` D8 (QA `LH-DATA-15-06`, `TC-1731`): the `allowed`
        // entry first, or the restore does not happen.
        audit_first(&self.pool, self.event("allowed", None), NOT_AUDITED_RESTORE).await?;
        let undropped = tokio::time::timeout(
            UNDROP_TIMEOUT,
            self.admin.undrop_table(&self.warehouse, &self.tabular),
        )
        .await;
        let undropped = match undropped {
            Ok(result) => result,
            Err(_elapsed) => {
                tracing::warn!(
                    table = %self.table,
                    "restore: the undrop did not answer within its bound; the outcome is not known"
                );
                Err(OpenfgaError::Transport)
            }
        };
        if let Err(err) = undropped {
            let (detail, refusal) = match err {
                // A race with a load that recreated the table between the check
                // and the undrop.
                OpenfgaError::Rejected => (
                    "the catalog refused the restore",
                    ApiError::Conflict(
                        "the catalog refused the restore; a table of that name may exist again"
                            .to_owned(),
                    ),
                ),
                // `DATA-55` review R2, the mirror case: an undrop that did not
                // answer may have happened, so ask once, with a bound, whether
                // the table is back.
                other => return self.settle_unknown_undrop(&other).await,
            };
            audit_after(&self.pool, self.event("failed", Some(detail))).await;
            return Err(refusal);
        }
        self.finish_restore().await;
        Ok(json!({ "ok": true }))
    }

    /// An undrop that did not answer: if the table is there, the restore
    /// happened — clear the record, write `executed`, answer as a success.
    /// Otherwise the record stays, `failed` is written and the fixed 503
    /// answers, as before (`DATA-55` review R2).
    async fn settle_unknown_undrop(&self, err: &OpenfgaError) -> Result<Value, ApiError> {
        let existing = tokio::time::timeout(
            super::catalog_drop::OUTCOME_CHECK_TIMEOUT,
            table_exists(&self.state, &self.table),
        )
        .await;
        if matches!(existing, Ok(Ok(true))) {
            self.finish_restore().await;
            return Ok(json!({ "ok": true }));
        }
        audit_after(
            &self.pool,
            self.event("failed", Some("the catalog did not answer")),
        )
        .await;
        Err(admin_unavailable(err))
    }

    /// The table is back: clear the drop's record (best effort) and write the
    /// `executed` entry, both inside the awaited task.
    async fn finish_restore(&self) {
        // `DATA-55` D8 (QA `LH-DATA-15-06`, `TC-1731`): the record of the drop
        // is cleared, because a table that is there is not dropped — but only
        // the record this restore read before the undrop went out (`DATA-55`
        // review U2). A record written since belongs to a drop of the same name
        // that may still be in flight, and removing it would leave that table
        // dropped with no record of it. Nothing read (or the read failed):
        // nothing is cleared, and the next read heals a leftover. A failure is
        // logged, not turned into a failed restore — the table IS back.
        if let Some(dropped_at) = self.dropped_at {
            match uploads::forget_table_drop_at(&self.pool, &self.table, dropped_at).await {
                Ok(true) => {}
                Ok(false) => tracing::warn!(
                    table = %self.table,
                    "restore: the record read for this restore is gone or was replaced; a record written since stays"
                ),
                Err(err) => {
                    tracing::warn!(?err, %self.table, "restore: the record of the drop could not be cleared");
                }
            }
        } else {
            tracing::warn!(
                table = %self.table,
                "restore: no record of the drop was read, so this restore clears none; the next read heals a leftover"
            );
        }
        audit_after(&self.pool, self.event("executed", None)).await;
    }

    /// The audit record of this restore, with this restore's own fields.
    fn event(&self, outcome: &str, detail: Option<&str>) -> NewAuditEvent {
        restore_event(
            &self.principal,
            &self.id,
            &self.table,
            &self.expires_at,
            outcome,
            detail,
        )
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use lakehouse_auth::openfga::DeletedTabular;

    fn dropped(expires_at: &str) -> DeletedTabular {
        DeletedTabular {
            id: "t-1".to_owned(),
            deleted_at: "2026-10-16T08:00:00Z".to_owned(),
            expires_at: expires_at.to_owned(),
        }
    }

    fn at(s: &str) -> OffsetDateTime {
        OffsetDateTime::parse(s, &Rfc3339).unwrap()
    }

    #[test]
    fn a_table_that_exists_is_not_restorable_even_with_a_dropped_copy() {
        let now = at("2026-10-17T00:00:00Z");
        let got = assess(
            true,
            DeletedTableLookup::Found(dropped("2026-10-23T08:00:00Z")),
            now,
        )
        .unwrap();
        assert_eq!(got, Assessment::Exists);
    }

    #[test]
    fn no_dropped_copy_is_nothing_to_restore() {
        let got = assess(false, DeletedTableLookup::None, at("2026-10-17T00:00:00Z")).unwrap();
        assert_eq!(got, Assessment::Nothing);
    }

    // PR #106 review R1 SHOULD-FIX DATA-15 2
    #[test]
    fn stopped_early_is_could_not_tell() {
        let got = assess(
            false,
            DeletedTableLookup::CouldNotTell,
            at("2026-10-17T00:00:00Z"),
        )
        .unwrap();
        assert_eq!(got, Assessment::CouldNotTell);
    }

    #[test]
    fn a_dropped_copy_inside_its_window_is_restorable_until_its_expiry() {
        let got = assess(
            false,
            DeletedTableLookup::Found(dropped("2026-10-23T08:00:00Z")),
            at("2026-10-17T00:00:00Z"),
        )
        .unwrap();
        assert_eq!(
            got,
            Assessment::Restorable {
                id: "t-1".to_owned(),
                dropped_at: "2026-10-16T08:00:00Z".to_owned(),
                expires_at: "2026-10-23T08:00:00Z".to_owned(),
            }
        );
    }

    #[test]
    fn a_dropped_copy_past_its_window_is_expired_not_restorable() {
        let got = assess(
            false,
            DeletedTableLookup::Found(dropped("2026-10-23T08:00:00Z")),
            at("2026-10-23T08:00:00Z"),
        )
        .unwrap();
        assert!(matches!(got, Assessment::Expired { .. }), "{got:?}");
    }

    #[test]
    fn an_unreadable_expiry_is_unavailable_not_a_guess() {
        let err = assess(
            false,
            DeletedTableLookup::Found(dropped("soon")),
            at("2026-10-17T00:00:00Z"),
        )
        .unwrap_err();
        assert_eq!(err.status(), 503);
    }
}
