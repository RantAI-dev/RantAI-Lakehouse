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

use axum::http::{HeaderMap, HeaderValue};
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use uuid::Uuid;

const X_TENANT_HEADER: &str = "x-tenant";

/// The one refusal every tenant answer that is not "yes" gets.
fn not_found() -> ApiError {
    ApiError::NotFound("tenant not found".to_owned())
}

/// The request headers a site acts as `tenant_id` with — the same
/// `X-Tenant` header a caller sends, built here so that a site which
/// acts as a tenant it was given (an authored pipeline acting as its
/// own tenant, `SEC-50`) is resolved by the SAME rule as a caller that
/// sent the header, never by a second one (`AGENTS.md` rule 4).
///
/// # Errors
///
/// [`ApiError::NotFound`] when the tenant id cannot be written as a
/// header value. It always can — a `Uuid` is ASCII — so this never
/// fires; the same fixed 404 as every other tenant refusal keeps it
/// harmless if it ever does.
pub(crate) fn headers_for_tenant(tenant_id: Uuid) -> Result<HeaderMap, ApiError> {
    let mut headers = HeaderMap::new();
    let value = HeaderValue::from_str(&tenant_id.to_string()).map_err(|_err| not_found())?;
    headers.insert(X_TENANT_HEADER, value);
    Ok(headers)
}

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

    use super::headers_for_tenant;
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

    /// `SEC-50`: the headers a site acts as a tenant with resolve the
    /// same way a caller's own header does.
    #[test]
    fn the_headers_of_a_named_tenant_resolve_as_that_tenant() {
        let principal = principal_with_tenants(&[TENANT_A, TENANT_B]);
        let headers = headers_for_tenant(TENANT_B).expect("a UUID is a valid header value");
        assert_eq!(resolve(&principal, &headers).unwrap(), Some(TENANT_B));
    }

    /// The same refusal as a caller's own header: a tenant the
    /// principal does not belong to is a 404, never a 403.
    #[test]
    fn the_headers_of_a_tenant_the_principal_left_are_refused() {
        let principal = principal_with_tenants(&[TENANT_A]);
        let headers = headers_for_tenant(TENANT_B).expect("a UUID is a valid header value");
        assert_eq!(resolve(&principal, &headers).unwrap_err().status(), 404);
    }
}
