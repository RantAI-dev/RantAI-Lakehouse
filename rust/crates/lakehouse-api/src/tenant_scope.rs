//! `X-Tenant`-scoping, fail closed. See the module's callers
//! (`routes::connectors`, `routes::pipelines`, `routes::lakehouse`): a
//! principal belonging to zero tenants must get an empty list, never a
//! 403 that would confirm a tenant exists.
//!
//! # The rule, in one sentence
//!
//! `resolve` never trusts `X-Tenant` beyond checking it against
//! `Principal::tenant_ids` — an id the header names that the principal does
//! not belong to is [`ApiError::NotFound`] (never
//! [`ApiError::Forbidden`]/403, which would confirm the tenant *exists* to a
//! caller who should not even learn that — a 403 leaks tenant existence, a
//! 404 does not), and a principal with no tenant at all resolves to
//! `Ok(None)`, which every caller of this function must treat as "return an
//! empty list," never "return every tenant's rows."
//!
//! The header is a SELECTION, not an authorization grant: it only narrows
//! which of the principal's own tenants is active for this request.
//! Membership itself is authorised server-side, entirely from
//! `Principal::tenant_ids` (populated at authentication time from
//! `app_user`/`service_identity` rows) — a caller cannot widen its own
//! access by sending a different header value, only ask to see a tenant it
//! is already a member of, or be refused.

use axum::http::HeaderMap;
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use uuid::Uuid;

const X_TENANT_HEADER: &str = "x-tenant";

/// Resolve the active tenant for this request.
///
/// - `X-Tenant` present and names a tenant `principal` belongs to: that
///   tenant.
/// - `X-Tenant` absent: the principal's first tenant, or `None` if it
///   belongs to none. A principal with exactly one tenant (the common case)
///   never needs to send the header at all.
/// - `X-Tenant` absent and the principal belongs to zero tenants: `None`,
///   which callers must render as an empty list, never as "no scoping, show
///   everything."
///
/// # Errors
///
/// [`ApiError::NotFound`] if `X-Tenant` is present but malformed (not valid
/// UTF-8, not a UUID), or names a tenant the principal does not belong to.
/// Both cases return the identical 404 body — a malformed header and a
/// real-but-foreign tenant id must be indistinguishable to the caller, or
/// the distinction itself becomes an oracle for "well-formed UUIDs that
/// exist" versus "well-formed UUIDs that don't."
pub fn resolve(principal: &Principal, headers: &HeaderMap) -> Result<Option<Uuid>, ApiError> {
    let not_found = || ApiError::NotFound("tenant not found".to_owned());
    match headers.get(X_TENANT_HEADER) {
        Some(value) => {
            let raw = value.to_str().map_err(|_err| not_found())?;
            let tenant_id: Uuid = raw.parse().map_err(|_err| not_found())?;
            if principal.in_tenant(tenant_id) {
                Ok(Some(tenant_id))
            } else {
                Err(not_found())
            }
        }
        None => Ok(principal.tenant_ids.first().copied()),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use axum::http::HeaderMap;
    use lakehouse_auth::{PermissionSet, Principal, PrincipalId};
    use uuid::Uuid;

    use super::resolve;

    const TENANT_A: Uuid = Uuid::from_u128(1);
    const TENANT_B: Uuid = Uuid::from_u128(2);

    fn principal_with_tenants(tenant_ids: &[Uuid]) -> Principal {
        Principal {
            id: PrincipalId::User(Uuid::nil()),
            tenant_ids: tenant_ids.to_vec(),
            display_name: "Test Principal".to_owned(),
            permissions: PermissionSet::parse("query:read"),
            provider: "local".to_owned(),
            must_change_password: false,
            role_names: vec!["Analyst".to_owned()],
        }
    }

    fn headers_with_x_tenant(tenant_id: Uuid) -> HeaderMap {
        headers_with_x_tenant_raw(&tenant_id.to_string())
    }

    fn headers_with_x_tenant_raw(raw: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("x-tenant", raw.parse().expect("test header value is ASCII"));
        headers
    }

    #[test]
    fn an_unknown_x_tenant_header_is_not_found_never_forbidden() {
        let principal = principal_with_tenants(&[TENANT_A]);
        let headers = headers_with_x_tenant(TENANT_B); // not in principal.tenant_ids
        let err = resolve(&principal, &headers).unwrap_err();
        assert_eq!(err.status(), 404); // never 403 — a 403 would confirm the tenant exists
    }

    #[test]
    fn no_header_and_no_tenants_resolves_to_none_meaning_show_nothing() {
        let principal = principal_with_tenants(&[]);
        let headers = HeaderMap::new();
        assert_eq!(resolve(&principal, &headers).unwrap(), None);
    }

    #[test]
    fn no_header_and_one_tenant_defaults_to_that_tenant() {
        let principal = principal_with_tenants(&[TENANT_A]);
        let headers = HeaderMap::new();
        assert_eq!(resolve(&principal, &headers).unwrap(), Some(TENANT_A));
    }

    #[test]
    fn a_valid_x_tenant_header_the_principal_belongs_to_is_honored() {
        let principal = principal_with_tenants(&[TENANT_A, TENANT_B]);
        let headers = headers_with_x_tenant(TENANT_B);
        assert_eq!(resolve(&principal, &headers).unwrap(), Some(TENANT_B));
    }

    #[test]
    fn a_malformed_x_tenant_header_is_not_found_not_a_parse_500() {
        let principal = principal_with_tenants(&[TENANT_A]);
        let headers = headers_with_x_tenant_raw("not-a-uuid");
        assert_eq!(resolve(&principal, &headers).unwrap_err().status(), 404);
    }
}
