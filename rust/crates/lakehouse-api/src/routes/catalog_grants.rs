//! Grant and revoke access from an asset's page (`DATA-15` F3).
//!
//! The Access tab already let a user request a permission and a
//! Governance Admin decide the request (`catalog::access_request`,
//! `catalog::decide_access_request`). These three routes add the other
//! half the spec asks for: a holder of `access:approve` grants one of the
//! requestable permissions to a user directly, sees the grants made from
//! the asset, and revokes one.
//!
//! # What a grant is
//!
//! The product's permissions are global: `query:read` reads every table, not
//! the one the grant was made from. A grant from this page therefore gives a
//! global permission for [`GRANT_DAYS`] days, and the page says so. The
//! asset only records where the grant was made, so the same page can list
//! and revoke it. A revoke takes effect at the grantee's next request,
//! because `lakehouse_auth::repository::load_principal_for_user` reads only
//! grants with `revoked_at IS NULL AND expires_at > now()` on every request.
//!
//! # Fail closed
//!
//! All three routes need `access:approve` (`POLICY_TABLE`) and run the
//! catalog tenant gate first, like the other catalog writes, before the body
//! is read. Nothing an upstream store says reaches a response: the store
//! errors are classified by `ApiError::from`.

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use serde::Deserialize;
use serde_json::{Value, json};

use super::catalog::{GRANT_DAYS, annotation_pool, catalog_tenant_refusal, validate_annotation_id};
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::state::AppState;

/// The permissions a grant from the asset page may carry: the three the
/// console lets a user request (`asset-detail-page.tsx`).
const GRANTABLE: [&str; 3] = ["catalog:write", "lineage:read", "query:read"];

/// The longest e-mail address the lookup accepts (RFC 5321's path limit).
const EMAIL_MAX_LEN: usize = 254;

/// `{ email, permission }` — the body of `POST /api/catalog/{id}/grants`.
#[derive(Debug, Deserialize)]
struct GrantBody {
    email: String,
    permission: String,
}

/// Best-effort audit write, like the other catalog writes: a failed audit
/// insert never fails the grant or revoke it describes.
async fn audit(
    pool: &lakehouse_store::PgPool,
    principal: &Principal,
    action: &str,
    asset: &str,
    args: Value,
) {
    let _ = lakehouse_store::audit::insert(
        pool,
        lakehouse_store::audit::NewAuditEvent {
            principal_id: Some(principal.id.uuid().to_string()),
            principal_kind: Some("user".to_owned()),
            actor_label: Some(principal.display_name.clone()),
            action: action.to_owned(),
            resource_kind: Some("catalog".to_owned()),
            resource_id: Some(asset.to_owned()),
            args: Some(args),
            outcome: "executed".to_owned(),
            ..Default::default()
        },
    )
    .await;
}

/// `GET /api/catalog/{id}/grants` — the grants in force that were made from
/// this asset (direct ones, and those of approved requests for it), newest
/// first: `{ grants: [{ id, user, userEmail, permission, grantedAt,
/// expiresAt, via, grantedBy }] }`.
///
/// # Errors
///
/// `403` with the tenant gate's fixed reason; `400` for an over-long id;
/// `503` without a Postgres pool; a classified store error otherwise.
pub async fn list(
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
    let grants = lakehouse_store::agents::list_asset_grants(pool, &id).await?;
    Ok(ApiJson(json!({ "grants": grants })))
}

/// `POST /api/catalog/{id}/grants` `{ email, permission }` — grants
/// `permission` to the user with that e-mail for [`GRANT_DAYS`] days.
///
/// # Errors
///
/// - `403` with the tenant gate's fixed reason, before the body is read.
/// - `400` for a body that is not JSON, an e-mail that is empty or too
///   long, or a `permission` that is not one of [`GRANTABLE`].
/// - `404` "no user with that email".
/// - `403` when the granter does not hold the permission themselves.
/// - `403` when the grantee is the caller (as approving one's own request
///   is refused).
/// - `403` when the grantee is outside the catalog tenant on a multi-tenant
///   install.
/// - `409` when the user already holds the permission, through a role or a
///   grant.
/// - `503` without a Postgres pool; a classified store error otherwise.
pub async fn create(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    if let Some(reason) = catalog_tenant_refusal(&state, &principal, &headers).await? {
        return Err(ApiError::PermissionDenied(reason.to_owned()).into());
    }
    validate_annotation_id(&id)?;
    let parsed: GrantBody = serde_json::from_slice(&body).map_err(|_| {
        ApiError::BadRequest("body must be JSON with an \"email\" and a \"permission\"".to_owned())
    })?;
    let email = parsed.email.trim();
    if email.is_empty() || email.len() > EMAIL_MAX_LEN {
        return Err(ApiError::BadRequest("enter the e-mail of a user".to_owned()).into());
    }
    let permission = parsed.permission.trim();
    if !GRANTABLE.contains(&permission) {
        return Err(ApiError::BadRequest(
            "permission must be catalog:write, lineage:read or query:read".to_owned(),
        )
        .into());
    }
    // PR #106 review R1 BLOCKER 4: a granter can only give permissions they themselves hold
    if !principal.has(permission) {
        return Err(ApiError::PermissionDenied(
            "you cannot grant a permission you do not hold".to_owned(),
        )
        .into());
    }
    let pool = annotation_pool(&state)?;
    let Some(target) = lakehouse_store::agents::find_user_by_email(pool, email).await? else {
        return Err(ApiError::NotFound("no user with that email".to_owned()).into());
    };
    // The server-side half of "nobody grants themselves access", as
    // `decide_access_request` refuses to approve one's own request.
    if target.id == principal.id.uuid() {
        return Err(
            ApiError::PermissionDenied("you cannot grant access to yourself".to_owned()).into(),
        );
    }
    // What the user can do now, from roles and grants alike: the same
    // loader every request uses, so "already holds it" and "can do it" agree.
    let held = lakehouse_auth::repository::load_principal_for_user(
        pool,
        target.id,
        "local".to_owned(),
        false,
    )
    .await
    .map_err(ApiError::from)?;
    if held.has(permission) {
        return Err(
            ApiError::Conflict("that user already holds this permission".to_owned()).into(),
        );
    }
    // PR #106 review R1 BLOCKER 5: grantee must belong to the catalog tenant when multiple tenants exist
    let (tenant_count,): (i64,) = sqlx::query_as("SELECT count(*) FROM tenant")
        .fetch_one(pool)
        .await
        .map_err(|_err| {
            ApiError::Unavailable(super::catalog::COULD_NOT_VERIFY_TENANT_COUNT.to_owned())
        })?;
    if tenant_count > 1
        && let Some(catalog_tenant_id) = state.config.catalog_tenant_id
        && !held.in_tenant(catalog_tenant_id)
    {
        return Err(ApiError::PermissionDenied(
            "grantee must be a member of the catalog tenant".to_owned(),
        )
        .into());
    }
    let grant = lakehouse_store::agents::create_direct_grant(
        pool,
        target.id,
        permission,
        &id,
        principal.id.uuid(),
        GRANT_DAYS,
    )
    .await?;
    // PR #106 review R1 SHOULD-FIX DATA-15 3: name grantee by display name + email
    audit(
        pool,
        &principal,
        "catalog.grant",
        &id,
        json!({
            "permission": permission,
            "grantee": format!("{} <{}>", target.name, email),
            "expiresAt": grant.expires_at,
        }),
    )
    .await;
    Ok(ApiJson(json!({
        "ok": true,
        "id": grant.id,
        "expiresAt": grant.expires_at,
    })))
}

/// `DELETE /api/catalog/{id}/grants/{grant_id}` — ends a grant made from
/// this asset. The grantee loses the permission at their next request.
///
/// # Errors
///
/// `403` with the tenant gate's fixed reason; `404` for a grant that does
/// not exist, was made from another asset, is already revoked or has
/// expired; `503` without a Postgres pool; a classified store error
/// otherwise.
pub async fn revoke(
    State(state): State<AppState>,
    Path((id, grant_id)): Path<(String, String)>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
) -> ApiResult<ApiJson<Value>> {
    if let Some(reason) = catalog_tenant_refusal(&state, &principal, &headers).await? {
        return Err(ApiError::PermissionDenied(reason.to_owned()).into());
    }
    validate_annotation_id(&id)?;
    let pool = annotation_pool(&state)?;
    let Some(revoked) =
        lakehouse_store::agents::revoke_asset_grant(pool, &id, &grant_id, principal.id.uuid())
            .await?
    else {
        return Err(ApiError::NotFound("no such grant in force".to_owned()).into());
    };
    audit(
        pool,
        &principal,
        "catalog.revoke",
        &id,
        json!({ "permission": revoked.permission, "grantee": revoked.user }),
    )
    .await;
    Ok(ApiJson(json!({ "ok": true })))
}
