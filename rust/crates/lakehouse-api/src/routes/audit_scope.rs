//! `SEC-20`: which rows of the audit trail one reader may see.
//!
//! The trail holds no tenant column — it is append-only
//! (`0142_audit_event_append_only.sql`) — so the tenant of a row is read
//! from its actor, at read time. This module is the ONE place that turns a
//! caller's reach into that rule, and it does it from the reach the rest of
//! the service already resolves: [`crate::routes::query::resolve_reach`],
//! which asks [`crate::routes::catalog::is_unrestricted`] and
//! [`crate::routes::catalog::catalog_tenant_refusal`]. The catalog-owner
//! rule therefore lives in one place, and moves with ADR 0018 when the
//! shared catalog does (`AGENTS.md` rule 4).

use axum::http::HeaderMap;
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use lakehouse_store::audit::AuditScope;

use crate::state::AppState;

/// The audit scope of a resolved reach.
///
/// A pure translation, so the rule can be read in one place: an
/// unrestricted caller reaches everything, a tenant reaches the rows of its
/// own actors (and the deployment's own rows when it owns the shared
/// catalog), and a caller that belongs to no tenant reaches nothing.
pub(crate) fn scope_for(reach: crate::routes::query::TenantReach) -> AuditScope {
    match reach {
        crate::routes::query::TenantReach::All => AuditScope::All,
        crate::routes::query::TenantReach::Tenant {
            tenant_id,
            owns_catalog,
        } => AuditScope::Tenant {
            tenant_id: tenant_id.to_string(),
            owns_catalog,
        },
        crate::routes::query::TenantReach::NoTenant => AuditScope::NoTenant,
    }
}

/// The audit scope of the caller, resolved once per request.
///
/// The reach comes from [`crate::routes::query::resolve_reach`], so this
/// reader and every other one that asks the same question get the same
/// answer.
///
/// # Errors
///
/// The errors of [`crate::routes::query::resolve_reach`]: the fail-closed
/// 404 for a forged `X-Tenant` header, and the 503 when the tenant count
/// cannot be read. Both narrow rather than widen: a reader that cannot
/// resolve its reach reads no row.
pub(crate) async fn scope_for_principal(
    state: &AppState,
    principal: &Principal,
    headers: &HeaderMap,
) -> Result<AuditScope, ApiError> {
    let reach = crate::routes::query::resolve_reach(state, principal, headers).await?;
    Ok(scope_for(reach))
}

/// The audit scope of a call with no person behind it.
///
/// A headless run belongs to no tenant, so it owns nobody's row and reads
/// no row of the trail. This is the fail-closed answer, and it is the same
/// one a caller of a deployment with several tenants and no membership
/// gets.
pub(crate) fn scope_without_principal() -> AuditScope {
    AuditScope::NoTenant
}
