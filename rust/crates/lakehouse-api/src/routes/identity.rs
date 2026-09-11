//! `/api/identity/*` — users, roles, tenants, and service identities,
//! backed by Postgres (`lakehouse-store`).
//!
//! # Not a port
//!
//! Every other module in `routes/` reproduces a TypeScript handler and is
//! held to the golden parity corpus. This one has no TypeScript ancestor:
//! it replaces `src/services/mock/identity.ts`, an *in-browser* mock that
//! never had a server side. There is therefore nothing to be bug-compatible
//! with, and the status codes below are chosen to be correct rather than
//! faithful: 201 on create, 404 on a missing row, 409 on a duplicate
//! natural key, 400 on a malformed body or an unknown role/tenant name,
//! 503 when there is no database pool at all.
//!
//! # The response bodies are the contract
//!
//! `src/services/contracts/identity.ts` is the spec, and it is not edited
//! by this task. The `lakehouse_store::identity` structs these handlers
//! return serialize to exactly the shapes it declares (camelCase keys,
//! `User[]`/`Role[]`/... as bare JSON arrays — not wrapped in an envelope,
//! because the contract's methods return arrays directly).
//!
//! # Authentication and authorization (Task 3.2, then D1)
//!
//! Every route in this module requires an authenticated
//! [`lakehouse_auth::Principal`] holding `identity:read` (list/get routes)
//! or `identity:write` (create routes) — see `crate::policy::POLICY_TABLE`.
//! Earlier, every route here was merely `Policy::RequiresAuth`: any
//! authenticated principal, not just an admin, could create users, mint a
//! role with `*:*` permissions, and attach that role to itself — a
//! privilege-escalation hole. `identity:read`/`identity:write` close it.
//! Neither token is granted by any seeded role's `role.permissions` value
//! except Platform Admin's `"*:*"` (which already satisfies both by the
//! resource-wildcard rule), so closing this hole required no seed-data
//! migration — see `crate::policy`'s module doc comment and
//! `identity_permissions_require_no_seed_change` for the proof.
//!
//! This means every handler below now runs behind `crate::policy::auth_gate`,
//! which itself requires `AppState::auth` (built only when
//! `AppState::pg` is `Some`) — so a Postgres outage now surfaces as 503
//! from the auth gate itself, before any handler's own database logic ever
//! runs. See `every_database_backed_route_returns_503_without_a_pool` for
//! the concrete proof, exercised end to end through the real router.

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::identity::{
    self, CreateRoleInput, CreateServiceIdentityInput, CreateTenantInput, InviteUserInput, Role,
    ServiceIdentity, ServiceIdentityFilter, Tenant, TenantFilter, User, UserFilter,
};
use serde::Deserialize;

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::state::AppState;

/// Borrow the Postgres pool, or fail with a 503 explaining why there isn't
/// one.
///
/// `AppState::pg` is `None` only when `DATABASE_URL` failed to parse at
/// startup (an unreachable Postgres still yields a pool — see
/// `lakehouse_store::connect_lazy`). That is a configuration problem, so
/// the message names the variable to fix rather than saying "internal
/// error", and the status is 503 (retry once the deployment is fixed)
/// rather than 500 (a bug here). Never panics: the whole point of the
/// `Option` is that Phase 1 routes keep serving when this is empty.
fn pool(state: &AppState) -> Result<&PgPool, ApiError> {
    state.pg.as_deref().ok_or_else(|| {
        ApiError::Unavailable(
            "identity store unavailable: no Postgres pool is configured \
             (DATABASE_URL is missing or not a valid Postgres connection string)"
                .to_owned(),
        )
    })
}

/// Parse a request body as JSON, reporting a parse failure as a 400 with
/// the parser's own message.
///
/// Mirrors `routes::alerts::parse_body` rather than using axum's `Json`
/// extractor: the extractor's rejection renders its own body shape, which
/// would be the one response in this crate not wrapped in
/// [`ApiJson`]/`{"error": ...}`.
fn parse_body<T: serde::de::DeserializeOwned>(body: &Bytes) -> Result<T, ApiError> {
    serde_json::from_slice(body).map_err(|err| ApiError::BadRequest(format!("invalid JSON: {err}")))
}

/// Reject a field that is empty or whitespace-only, returning the trimmed
/// value otherwise.
///
/// The database's `NOT NULL` says nothing about `""`, and a nameless
/// tenant or an empty email is a caller mistake (400), not something to
/// persist and render in the console forever.
fn required(field: &str, value: &str) -> Result<String, ApiError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ApiError::BadRequest(format!("{field} is required")));
    }
    Ok(trimmed.to_owned())
}

// ── Users ───────────────────────────────────────────────────────────────

/// Query parameters for `GET /api/identity/users`.
///
/// The contract's `listUsers()` takes no arguments, so the console never
/// sends these; they exist because the repository supports them and a
/// filtered list is cheaper server-side than shipping every user to the
/// browser when a caller does want one.
#[derive(Debug, Deserialize)]
pub struct UserQuery {
    /// `?status=active|inactive`.
    status: Option<String>,
    /// `?tenant=<slug>` — only users belonging to that tenant.
    tenant: Option<String>,
}

/// `GET /api/identity/users` — the workspace user directory.
///
/// # Errors
///
/// 503 if no pool is configured; 500 on a database failure.
pub async fn list_users(
    State(state): State<AppState>,
    Query(query): Query<UserQuery>,
) -> ApiResult<ApiJson<Vec<User>>> {
    let filter = UserFilter {
        status: query.status,
        tenant_slug: query.tenant,
    };
    Ok(ApiJson(identity::list_users(pool(&state)?, &filter).await?))
}

/// The `POST /api/identity/users` body. Mirrors `InviteUserInput` in
/// `contracts/identity.ts`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteUserBody {
    /// Display name.
    name: String,
    /// Email address.
    email: String,
    /// Role names to grant. Optional in the wire format (defaults to none)
    /// so an invite with no roles doesn't have to send `[]`.
    #[serde(default)]
    roles: Vec<String>,
    /// Tenant names to join.
    #[serde(default)]
    tenants: Vec<String>,
}

/// `POST /api/identity/users` — invite a user. Returns 201 with the created
/// [`User`].
///
/// # Security
///
/// Requires `identity:write` — see the module doc comment.
///
/// # Errors
///
/// 400 on a malformed body, a blank name/email, or an unknown role/tenant
/// name; 409 if the email is already registered; 503/500 as above.
pub async fn create_user(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<User>)> {
    let body: InviteUserBody = parse_body(&body)?;
    let input = InviteUserInput {
        name: required("name", &body.name)?,
        email: required("email", &body.email)?,
        roles: body.roles,
        tenants: body.tenants,
    };
    let user = identity::create_user(pool(&state)?, &input).await?;
    Ok((StatusCode::CREATED, ApiJson(user)))
}

// ── Roles ───────────────────────────────────────────────────────────────

/// `GET /api/identity/roles` — every role, with its derived member count.
///
/// # Errors
///
/// 503 if no pool is configured; 500 on a database failure.
pub async fn list_roles(State(state): State<AppState>) -> ApiResult<ApiJson<Vec<Role>>> {
    Ok(ApiJson(identity::list_roles(pool(&state)?).await?))
}

/// The `POST /api/identity/roles` body. Mirrors `CreateRoleInput`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRoleBody {
    /// Role name.
    name: String,
    /// Free-text permission list.
    #[serde(default)]
    permissions: String,
    /// Human-readable description.
    #[serde(default)]
    description: String,
}

/// `POST /api/identity/roles` — create a role. Returns 201.
///
/// # Security
///
/// Requires `identity:write` — see the module doc comment. This one is the
/// sharpest of the three creates: it mints a permission bundle (`"*:*"` is
/// a legal value) that a subsequent invite can attach to a user, which is
/// exactly the escalation path `identity:write` exists to gate.
///
/// # Errors
///
/// 400 on a malformed body or a blank name; 409 if the name is taken;
/// 503/500 as above.
pub async fn create_role(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<Role>)> {
    let body: CreateRoleBody = parse_body(&body)?;
    let input = CreateRoleInput {
        name: required("name", &body.name)?,
        permissions: body.permissions,
        description: body.description,
    };
    let role = identity::create_role(pool(&state)?, &input).await?;
    Ok((StatusCode::CREATED, ApiJson(role)))
}

// ── Tenants ─────────────────────────────────────────────────────────────

/// Query parameters for `GET /api/identity/tenants`.
#[derive(Debug, Deserialize)]
pub struct TenantQuery {
    /// `?plan=Enterprise` — only tenants on that plan.
    plan: Option<String>,
}

/// `GET /api/identity/tenants` — every tenant, with its derived user count.
///
/// # Errors
///
/// 503 if no pool is configured; 500 on a database failure.
pub async fn list_tenants(
    State(state): State<AppState>,
    Query(query): Query<TenantQuery>,
) -> ApiResult<ApiJson<Vec<Tenant>>> {
    let filter = TenantFilter { plan: query.plan };
    Ok(ApiJson(
        identity::list_tenants(pool(&state)?, &filter).await?,
    ))
}

/// The `POST /api/identity/tenants` body. Mirrors `CreateTenantInput`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTenantBody {
    /// Display name.
    name: String,
    /// URL-safe identifier.
    slug: String,
    /// Plan name.
    #[serde(default)]
    plan: String,
    /// Residency policy description.
    #[serde(default)]
    residency: String,
}

/// `POST /api/identity/tenants` — create a tenant. Returns 201.
///
/// # Security
///
/// Requires `identity:write` — see the module doc comment.
///
/// # Errors
///
/// 400 on a malformed body or a blank name/slug; 409 if the slug is taken;
/// 503/500 as above.
pub async fn create_tenant(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<Tenant>)> {
    let body: CreateTenantBody = parse_body(&body)?;
    let input = CreateTenantInput {
        name: required("name", &body.name)?,
        slug: required("slug", &body.slug)?,
        plan: body.plan,
        residency: body.residency,
    };
    let tenant = identity::create_tenant(pool(&state)?, &input).await?;
    Ok((StatusCode::CREATED, ApiJson(tenant)))
}

// ── Service identities ──────────────────────────────────────────────────

/// Query parameters for `GET /api/identity/service-identities`.
#[derive(Debug, Deserialize)]
pub struct ServiceIdentityQuery {
    /// `?environment=production` — only identities in that environment.
    environment: Option<String>,
}

/// `GET /api/identity/service-identities` — non-human principals.
///
/// Returns credential *metadata* only (name, scopes, environment, expiry,
/// rotation status, last use). No secret material is stored by the schema,
/// so none can be returned here.
///
/// # Errors
///
/// 503 if no pool is configured; 500 on a database failure.
pub async fn list_service_identities(
    State(state): State<AppState>,
    Query(query): Query<ServiceIdentityQuery>,
) -> ApiResult<ApiJson<Vec<ServiceIdentity>>> {
    let filter = ServiceIdentityFilter {
        environment: query.environment,
    };
    Ok(ApiJson(
        identity::list_service_identities(pool(&state)?, &filter).await?,
    ))
}

/// The `POST /api/identity/service-identities` body. Mirrors
/// `CreateServiceIdentityInput`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateServiceIdentityBody {
    /// Credential name.
    name: String,
    /// Granted scopes.
    #[serde(default)]
    scopes: Vec<String>,
    /// Deployment environment.
    #[serde(default)]
    environment: String,
}

/// `POST /api/identity/service-identities` — register a service identity.
/// Returns 201.
///
/// Records that a credential exists; it does not issue one (the contract
/// has no field for a secret, and the schema has no column for it).
///
/// # Security
///
/// Requires `identity:write` — see the module doc comment.
///
/// # Errors
///
/// 400 on a malformed body or a blank name/environment; 409 if the name is
/// taken; 503/500 as above.
pub async fn create_service_identity(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<ServiceIdentity>)> {
    let body: CreateServiceIdentityBody = parse_body(&body)?;
    let input = CreateServiceIdentityInput {
        name: required("name", &body.name)?,
        scopes: body.scopes,
        environment: required("environment", &body.environment)?,
    };
    let created = identity::create_service_identity(pool(&state)?, &input).await?;
    Ok((StatusCode::CREATED, ApiJson(created)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use serde_json::Value;

    use super::*;
    use crate::config::Config;

    /// A state whose `pg` is `None`, the one case [`pool`] exists to
    /// handle: `DATABASE_URL` that cannot be parsed at all.
    fn state_without_pool() -> AppState {
        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), "not a postgres url".to_owned());
        AppState::new(Config::from_map(&env).unwrap())
    }

    /// With no pool, identity routes must answer 503 with a message that
    /// names the thing to fix — never panic, and never claim a 500-style
    /// internal error for what is a configuration problem.
    #[tokio::test]
    async fn missing_pool_is_a_503_naming_database_url() {
        let state = state_without_pool();
        let err = pool(&state).expect_err("a malformed DATABASE_URL must yield no pool");
        assert_eq!(err.status(), 503);
        assert!(
            err.to_string().contains("DATABASE_URL"),
            "message should name the misconfigured variable, got: {err}"
        );
    }

    /// Every identity handler that touches the database must go through
    /// [`pool`], so none of them can panic when Postgres was never
    /// configured. Exercised end to end through the real router.
    #[tokio::test]
    async fn every_database_backed_route_returns_503_without_a_pool() {
        use axum::body::to_bytes;
        use axum::http::Request;
        use tower::ServiceExt;

        let paths = [
            "/api/identity/users",
            "/api/identity/roles",
            "/api/identity/tenants",
            "/api/identity/service-identities",
        ];
        for path in paths {
            let app = crate::routes::router(state_without_pool());
            let response = app
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::SERVICE_UNAVAILABLE,
                "{path} should be 503 without a pool"
            );
            let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let body: Value = serde_json::from_slice(&bytes).unwrap();
            assert!(
                body.get("error").is_some(),
                "{path} should use the {{\"error\": ...}} envelope"
            );
        }
    }

    /// A blank or whitespace-only required field is the caller's mistake,
    /// not a row to persist.
    #[test]
    fn required_rejects_blank_and_trims() {
        assert_eq!(required("name", "  Rina  ").unwrap(), "Rina");
        let err = required("name", "   ").expect_err("whitespace-only must be rejected");
        assert_eq!(err.status(), 400);
    }

    /// A malformed body is a 400 with the parser's message, not a 500 and
    /// not axum's own non-envelope rejection body.
    #[test]
    fn parse_body_reports_bad_json_as_400() {
        let err = parse_body::<CreateRoleBody>(&Bytes::from_static(b"{not json"))
            .expect_err("malformed JSON must be rejected");
        assert_eq!(err.status(), 400);
    }
}
