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

use std::sync::Arc;

use axum::Extension;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use iceberg::{NamespaceIdent, TableIdent};
use lakehouse_auth::Principal;
use lakehouse_auth::openfga::{DeletedTableLookup, LakekeeperAdminClient, OpenfgaError};
use lakehouse_core::ApiError;
use lakehouse_iceberg::rest::{self, RestError};
use serde_json::{Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::catalog::{annotation_pool, catalog_tenant_refusal, validate_annotation_id};
use super::catalog_profile::bronze_table_name;
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::lakehouse_catalog::{self, CatalogAccessError};
use crate::state::AppState;

/// The namespace Bronze datasets' Iceberg tables live in.
const BRONZE_NAMESPACE: &str = "bronze";

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
async fn table_exists(state: &AppState, table: &str) -> Result<bool, ApiError> {
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
/// # Errors
///
/// `403` with the tenant gate's reason; `404` when nothing is restorable (or
/// the asset has no restorable kind of table); `409` when a table of that
/// name exists again; `410` when the 7-day window has passed; `503` when
/// restore is not configured or `Lakekeeper` cannot be asked.
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
    let pool = annotation_pool(&state)?;
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
    let warehouse = warehouse.ok_or_else(|| ApiError::Unavailable(UNREACHABLE.to_owned()))?;
    admin
        .undrop_table(&warehouse, &tabular)
        .await
        .map_err(|err| match err {
            // A race with a load that recreated the table between the check
            // and the undrop.
            OpenfgaError::Rejected => ApiError::Conflict(
                "the catalog refused the restore; a table of that name may exist again".to_owned(),
            ),
            other => admin_unavailable(&other),
        })?;
    let _ = lakehouse_store::audit::insert(
        pool,
        lakehouse_store::audit::NewAuditEvent {
            principal_id: Some(principal.id.uuid().to_string()),
            principal_kind: Some("user".to_owned()),
            actor_label: Some(principal.display_name.clone()),
            action: "catalog.restore".to_owned(),
            resource_kind: Some("catalog".to_owned()),
            resource_id: Some(id),
            args: Some(json!({ "table": table, "wasRestorableUntil": expires_at })),
            outcome: "executed".to_owned(),
            ..Default::default()
        },
    )
    .await;
    Ok(ApiJson(json!({ "ok": true })))
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
