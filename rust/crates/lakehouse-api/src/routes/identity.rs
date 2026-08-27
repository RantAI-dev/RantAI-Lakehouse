//! `/api/identity/*` — users, roles, tenants, service identities, and
//! workspace settings, backed by Postgres (`lakehouse-store`).
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
//! # No authentication, deliberately
//!
//! There is no auth layer anywhere in this service, and this task does not
//! add one — see the task's scope. Every endpoint below is reachable by
//! anyone who can reach the port, including the three `POST`s that create
//! real directory rows. That is a known, escalated product gap, not an
//! oversight of this module; it is called out again on each mutating
//! handler.

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::identity::{
    self, CreateRoleInput, CreateServiceIdentityInput, CreateTenantInput, InviteUserInput, Role,
    ServiceIdentity, ServiceIdentityFilter, Tenant, TenantFilter, User, UserFilter,
};
use serde::{Deserialize, Serialize};

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
        return Err(ApiError::BadRequest(format!("{field} wajib diisi")));
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
/// Unauthenticated, like every route in this service: any caller who can
/// reach this port can add a person to the directory and grant them roles
/// in any tenant. See the module doc comment.
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
/// Unauthenticated — see the module doc comment. This one is the sharpest
/// of the three creates: it mints a permission bundle (`"*:*"` is a legal
/// value) that a subsequent invite can attach to a user.
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
/// Unauthenticated — see the module doc comment.
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
/// Unauthenticated — see the module doc comment.
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

// ── Workspace settings ──────────────────────────────────────────────────

/// Mirrors `WorkspaceSettings` in `contracts/identity.ts`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSettings {
    /// Display name of the workspace.
    workspace_name: &'static str,
    /// Environment new work defaults to.
    default_environment: &'static str,
    /// Slug of the tenant the console opens in.
    default_tenant: &'static str,
    /// `"dark" | "light" | "system"`.
    interface_theme: &'static str,
    /// `"mock" | "http"` — which service adapter the console is running
    /// against.
    service_adapter: &'static str,
    /// Audit retention, in days.
    audit_retention_days: u32,
    /// Query-result retention, in days.
    query_result_retention_days: u32,
}

/// `GET /api/identity/workspace-settings` — workspace-wide console
/// configuration.
///
/// # Why this one is not Postgres-backed
///
/// `0001_init.sql` deliberately left `WorkspaceSettings` out of the schema,
/// and that call still holds: the contract exposes it as a *read-only*
/// getter (there is no `updateWorkspaceSettings`), so there is nothing a
/// table would let a caller do that a constant does not. Adding a
/// singleton-row table plus a migration to serve seven values nothing can
/// change would be schema for its own sake. It is app configuration, not
/// identity data — a later task that actually introduces an editable
/// settings screen is the one that should give it a home.
///
/// The values are the mock's, with one deliberate correction:
/// `serviceAdapter` reports `"http"`, not `"mock"`. That field names the
/// adapter the console is actually talking to, and as of this task that is
/// the real HTTP backend — reporting `"mock"` from the real backend would
/// be a self-contradicting response.
///
/// Serving this from the process rather than the database also means it
/// keeps working when `pool` would 503, which is the right behavior for a
/// value the console reads on boot to decide how to render itself.
pub async fn workspace_settings() -> ApiJson<WorkspaceSettings> {
    ApiJson(WorkspaceSettings {
        workspace_name: "Rantai Lake",
        default_environment: "production",
        default_tenant: "meridian-group",
        interface_theme: "dark",
        service_adapter: "http",
        audit_retention_days: 365,
        query_result_retention_days: 30,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use serde_json::{Value, json};

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

    /// Workspace settings are deliberately NOT database-backed, so they
    /// must still answer 200 when every other identity route is 503-ing.
    #[tokio::test]
    async fn workspace_settings_serve_without_a_database() {
        use axum::body::to_bytes;
        use axum::http::Request;
        use axum::response::IntoResponse;
        use tower::ServiceExt;

        let app = crate::routes::router(state_without_pool());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/identity/workspace-settings")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let bytes = to_bytes(
            workspace_settings().await.into_response().into_body(),
            usize::MAX,
        )
        .await
        .unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            body,
            json!({
                "workspaceName": "Rantai Lake",
                "defaultEnvironment": "production",
                "defaultTenant": "meridian-group",
                "interfaceTheme": "dark",
                "serviceAdapter": "http",
                "auditRetentionDays": 365,
                "queryResultRetentionDays": 30
            }),
            "keys must match `WorkspaceSettings` in contracts/identity.ts exactly"
        );
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
