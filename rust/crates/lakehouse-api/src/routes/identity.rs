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
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use lakehouse_auth::Principal;
use lakehouse_auth::openfga::LakekeeperAdminClient;
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::audit::{self as store_audit, NewAuditEvent};
use lakehouse_store::identity::{
    self, CreateRoleInput, CreateServiceIdentityInput, CreateTenantInput, InviteUserInput, Role,
    RotateServiceIdentityResponse, ServiceIdentity, ServiceIdentityFilter, Tenant, TenantFilter,
    User, UserFilter,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::AuthenticatedPrincipal;
use crate::config::Config;
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

/// `POST /api/identity/tenants` — create (or resume provisioning of) a
/// tenant.
///
/// # Security
///
/// Requires `identity:write` — see the module doc comment.
///
/// # Resumable provisioning (WS8 plan Task B4, Correction 7)
///
/// A tenant's Lakekeeper warehouse, grants, and registry namespace are
/// provisioned by three external calls this handler drives as a
/// checkpointed state machine (`tenant.provisioning_status`,
/// `0042_tenant_provisioning.sql`). A second `POST` of the same `slug`
/// does not restart from zero or 409 outright: it finds the existing row
/// and, if it is still genuinely in progress
/// (`pending`/`warehouse_ready`/`grants_ready`/`namespace_ready`),
/// resumes from the first incomplete step — using
/// [`LakekeeperAdminClient::ensure_warehouse`]'s own lookup-before-create
/// before ever attempting a Lakekeeper create, so a resumed run never
/// double-provisions. A row already `complete` or `not_applicable` (a
/// tenant seeded before provisioning existed at all —
/// `0042_tenant_provisioning.sql`'s backfill) is a genuine 409: neither
/// status is a step to resume from.
///
/// # Errors
///
/// 400 on a malformed body or a blank name/slug; 409 if the slug belongs
/// to a tenant already `complete` or `not_applicable`; 503 if no Postgres
/// pool is configured, or if a provisioning step could not reach
/// Lakekeeper (never claims success while leaving a half-provisioned row);
/// 500 on any other database failure.
/// Provisioning statuses (`tenant.provisioning_status`) that a repeated
/// `POST /api/identity/tenants` for the same slug resumes from, rather
/// than rejecting with 409 — see [`create_tenant`]'s doc comment and WS8
/// plan Correction 7. Deliberately excludes `complete` and
/// `not_applicable`: neither is a step to resume from.
const RESUMABLE_STATUSES: &[&str] = &[
    "pending",
    "warehouse_ready",
    "grants_ready",
    "namespace_ready",
];

pub async fn create_tenant(
    State(state): State<AppState>,
    AuthenticatedPrincipal(principal): AuthenticatedPrincipal,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<Tenant>)> {
    let body: CreateTenantBody = parse_body(&body)?;
    let name = required("name", &body.name)?;
    let slug = required("slug", &body.slug)?;
    let pool = pool(&state)?;

    // Resume, don't reject, a slug that already exists but is still
    // mid-provisioning (WS8 plan Correction 7). A 409 is reserved for a
    // slug already terminal — either genuinely `complete`, or
    // `not_applicable` (a grandfathered pre-provisioning tenant,
    // `0042_tenant_provisioning.sql`'s backfill: re-POSTing its slug must
    // not silently pull a legacy row into a provisioning attempt nobody
    // asked for).
    let (tenant, status_code) = match identity::find_tenant_by_slug(pool, &slug).await? {
        Some(existing) if RESUMABLE_STATUSES.contains(&existing.provisioning_status.as_str()) => {
            (existing, StatusCode::OK)
        }
        Some(_) => return Err(ApiError::Conflict("slug already taken".to_owned()).into()),
        None => {
            let input = CreateTenantInput {
                name,
                slug: slug.clone(),
                plan: body.plan,
                residency: body.residency,
            };
            (
                identity::create_tenant(pool, &input).await?,
                StatusCode::CREATED,
            )
        }
    };

    // audit_event row mirrors WS5's audit schema — `outcome = "executed"`
    // per `audit_event_outcome_check` (0024_audit_event.sql). Written
    // BEFORE the `lakekeeper_admin` check so the audit fires whether or
    // not provisioning is reachable on this deployment — the tenant row
    // already exists in either branch (resume or fresh create), so an
    // operator's history is honest about what happened. Best-effort: a
    // failed audit write is logged and swallowed, never propagated, so
    // the response code on the wire is always the underlying operation's,
    // not the audit's (WS8 §Phase G).
    let audit_event = tenant_create_audit_event(&principal, &tenant);
    if let Err(err) = store_audit::insert(pool, audit_event).await {
        tracing::warn!(
            %err,
            action = "identity.tenant.create",
            tenant_id = %tenant.id,
            "failed to record tenant-create audit event"
        );
    }

    let Some(admin) = state.lakekeeper_admin.as_ref() else {
        // Provisioning is unreachable, not silently skipped — the tenant
        // row exists (or already did) but stays at whatever status it
        // had; a caller sees `provisioningStatus != "complete"` and knows
        // to retry once Lakekeeper admin access is configured on this
        // deployment, rather than being told the create succeeded when a
        // warehouse was never provisioned.
        return Ok((status_code, ApiJson(tenant)));
    };

    let tenant = provision_tenant(pool, admin, tenant, &state.config).await?;
    Ok((status_code, ApiJson(tenant)))
}

/// Drives `tenant` through the provisioning state machine from wherever it
/// currently sits, persisting the checkpoint after each step so a crash
/// mid-way leaves an honest, resumable `provisioning_status` rather than a
/// silent gap or a fabricated `complete`.
///
/// Each `if` below only fires when `tenant.provisioning_status` is still
/// at the step it names — after a step's own
/// `update_tenant_provisioning_status` call advances it, later `if`s in
/// the same invocation continue straight through (a brand-new tenant runs
/// all four in one call), and a resumed tenant simply skips every `if`
/// whose step it has already passed.
///
/// # Errors
///
/// [`crate::error::provisioning_unavailable`] (503) if a Lakekeeper call
/// fails — never `err.to_string()`, per AGENTS.md's "upstream error text
/// never reaches a response". A [`lakehouse_core::StoreError`] from a
/// checkpoint write propagates as-is via `?`.
async fn provision_tenant(
    pool: &PgPool,
    admin: &LakekeeperAdminClient,
    mut tenant: Tenant,
    config: &Config,
) -> ApiResult<Tenant> {
    let warehouse_name = format!("tenant-{}", tenant.slug);
    let prefix = format!("{}/{}/", config.lakehouse_warehouse_bucket, tenant.slug);

    if tenant.provisioning_status == "pending" {
        let warehouse_id = admin
            .ensure_warehouse(&warehouse_name, &prefix)
            .await
            .map_err(|_| crate::error::provisioning_unavailable())?;
        tenant = identity::update_tenant_provisioning_status(
            pool,
            &tenant.id,
            "warehouse_ready",
            Some(&warehouse_id),
        )
        .await?;
    }
    if tenant.provisioning_status == "warehouse_ready" {
        let Some(warehouse_id) = tenant.warehouse_id.clone() else {
            // Can only happen if a row was hand-edited into
            // `warehouse_ready` without a `warehouse_id` — the state
            // machine itself never advances a tenant to this status
            // without recording one in the same write (see the branch
            // above). A 400 rather than a panic: this is a malformed row,
            // not this handler's bug.
            return Err(ApiError::BadRequest(
                "tenant has no warehouse_id to grant onto".to_owned(),
            )
            .into());
        };
        admin
            .grant_machine_principals(&warehouse_id)
            .await
            .map_err(|_| crate::error::provisioning_unavailable())?;
        tenant =
            identity::update_tenant_provisioning_status(pool, &tenant.id, "grants_ready", None)
                .await?;
    }
    // Registry namespace: the grand plan calls for creating this tenant's
    // Iceberg namespace here, and this state machine reserves
    // `namespace_ready`/`complete` for it — but neither status is written
    // yet, because no namespace is created yet. `lakehouse-iceberg`'s only
    // namespace-create surface (`ensure_bronze_namespace`/
    // `ensure_gold_namespace`) is hardcoded to this deployment's one shared
    // catalog, not parameterized by a tenant warehouse, so calling it would
    // create a namespace on the WRONG catalog; inventing a second,
    // tenant-scoped Iceberg client here is out of this task's scope.
    //
    // So provisioning STOPS at `grants_ready`, and the caller is told that
    // by the status it reads back. Advancing to `namespace_ready` and then
    // `complete` without a namespace call would put a fabricated completion
    // in the database — the same untruth the `not_applicable` status exists
    // to avoid for grandfathered tenants (migration 0042's own header), and
    // the one a code comment cannot fix, because the operator reading
    // `provisioningStatus` never sees the comment.
    //
    // A warehouse exists and this stack's machine principals can write into
    // it, which is what the two completed steps claim. Finishing the last
    // step needs a per-tenant namespace API in `lakehouse-iceberg` first.
    Ok(tenant)
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

/// `POST /api/identity/service-identities/{id}/rotate` — revoke every
/// currently-unrevoked credential for `identity_id`, mint a fresh
/// credential, reset `expires_at`/`rotation_status`, audit, and return the
/// new raw token exactly once.
///
/// # Security
///
/// Requires `identity:write` — see the module doc comment. A leaked old
/// token is dead the moment this route returns 200, because every
/// previously-unrevoked `service_credential` row is set `revoked_at = now()`
/// inside the same transaction that inserts the new one
/// (`lakehouse_store::identity::rotate_service_identity`); a token replayed
/// after rotation therefore cannot match `service_credential.token_hash`
/// with `revoked_at IS NULL`, and `lakehouse_auth::service_token::
/// verify_service_token` filters on exactly that predicate.
///
/// # The secret is returned EXACTLY ONCE on this response
///
/// The raw token in `response.secret` is the value whose SHA-256 hex digest
/// was just persisted to `service_credential.token_hash`. It is never
/// persisted in plaintext (only the hash is, by design — see
/// `lakehouse-auth::token`'s doc comment), and it is never re-served by
/// any other route: `GET /api/identity/service-identities` and
/// `GET /api/identity/service-identities/{id}` return credential
/// *metadata* (`name`, `scopes`, `expires_at`, `rotation_status`) only,
/// because `ServiceIdentity` itself has no `secret`/`token`/`token_hash`
/// field. The console's caller is responsible for recording the secret at
/// this moment — the server never sees it again.
///
/// # Audit is best-effort
///
/// The `audit_event` write is wrapped in `if let Err ... tracing::warn!`,
/// mirroring `routes::agents::decide_approval`'s own pattern: a successful
/// rotation is still 200 with the secret if the audit insert fails — the
/// audit failure is operator-visible through the log line and never
/// propagated into the response, per AGENTS.md principle 4 (no
/// `ApiError::Internal` interpolating an upstream error).
///
/// # Errors
///
/// 400 on a non-UUID `{id}` (a malformed path can't match any row, so
/// 404'ing it would conflate "doesn't exist" with "caller error" — the
/// same posture as `routes::auth::revoke_session`'s 400 for a non-UUID
/// session id); 404 via [`StoreError::NotFound`] when `{id}` is well-formed
/// but no row matches (the final `get_service_identity` re-read is what
/// surfaces that — see `rotate_service_identity`'s doc comment); 503 if no
/// Postgres pool is configured; 500 (classified) on any other storage
/// failure, with the upstream error text never reaching the response.
pub async fn rotate_service_identity(
    State(state): State<AppState>,
    AuthenticatedPrincipal(principal): AuthenticatedPrincipal,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<RotateServiceIdentityResponse>> {
    let identity_id = Uuid::parse_str(&id)
        .map_err(|_| ApiError::BadRequest("service identity id must be a UUID".to_owned()))?;

    let pool = pool(&state)?;
    let response = identity::rotate_service_identity(pool, identity_id, || {
        // `mint_credential` returns the token together with the hash
        // `verify_service_token` will look it up by — the store function
        // takes a mint callback because it runs the revoke-and-insert in its
        // own transaction and cannot depend on `lakehouse-auth`. The closure
        // runs inside that transaction, so a failure anywhere rolls the
        // whole rotation back rather than orphaning the new credential.
        let (token, token_hash) = lakehouse_auth::service_token::mint_credential();
        (token.expose().to_owned(), token_hash)
    })
    .await?;

    let audit_event = rotate_service_identity_audit_event(&principal, identity_id);
    if let Err(err) = store_audit::insert(pool, audit_event).await {
        tracing::warn!(
            %err,
            action = "identity.service_identity.rotate",
            identity_id = %identity_id,
            "failed to record service-identity rotation audit event"
        );
    }

    Ok(ApiJson(response))
}

/// Build the `NewAuditEvent` for a successful rotate — extracted as a pure,
/// unit-tested function so the field-shape assertion lives next to the call
/// site (rather than a brittle source-text grep), exactly the pattern
/// `routes::agents::decide_approval_audit_event` and
/// `routes::query::query_run_audit_event` use. The audit row is
/// `outcome = "executed"` because rotation is a successful write (not a
/// gate decision) — the schema's `audit_event_outcome_check` allows it.
/// `principal_kind` comes from `Principal::kind_for_audit`, not from
/// `principal.provider` — the two are different vocabularies (the audit
/// CHECK accepts `"user" | "service" | "copilot" | "schedule"`; the
/// authenticator's `provider` field is `"local" | "session" | "service" |
/// "oidc:<issuer>"`), and binding `provider` here would write an
/// audit-invalid `principal_kind` for every human caller.
fn rotate_service_identity_audit_event(principal: &Principal, identity_id: Uuid) -> NewAuditEvent {
    NewAuditEvent {
        action: "identity.service_identity.rotate".to_owned(),
        resource_kind: Some("service_identity".to_owned()),
        resource_id: Some(identity_id.to_string()),
        principal_id: Some(principal.id.uuid().to_string()),
        principal_kind: Some(principal.kind_for_audit().to_owned()),
        actor_label: Some(principal.display_name.clone()),
        outcome: "executed".to_owned(),
        args: None,
        detail: None,
        ..Default::default()
    }
}

/// Build the `NewAuditEvent` for a successful tenant create — same
/// pattern as [`rotate_service_identity_audit_event`]: pure,
/// unit-testable, field-shape lives next to the call site. The audit
/// row is `outcome = "executed"` because create is a successful write —
/// the schema's `audit_event_outcome_check` allows it. `principal_kind`
/// comes from `Principal::kind_for_audit`, not from `principal.provider`
/// (the two are different vocabularies — see `Principal::kind_for_audit`'s
/// own doc comment). `args` carries the slug and provisioning status
/// the operator would otherwise have to re-query the row to recover.
fn tenant_create_audit_event(principal: &Principal, tenant: &Tenant) -> NewAuditEvent {
    NewAuditEvent {
        action: "identity.tenant.create".to_owned(),
        resource_kind: Some("tenant".to_owned()),
        resource_id: Some(tenant.id.clone()),
        principal_id: Some(principal.id.uuid().to_string()),
        principal_kind: Some(principal.kind_for_audit().to_owned()),
        actor_label: Some(principal.display_name.clone()),
        outcome: "executed".to_owned(),
        args: Some(serde_json::json!({
            "slug": tenant.slug,
            "provisioningStatus": tenant.provisioning_status,
        })),
        ..Default::default()
    }
}

#[cfg(test)]
mod rotated_credential_authenticates {
    //! The one property rotation exists for: the secret it returns must
    //! actually authenticate, and the credentials it replaced must not.
    //! Lives here because this crate depends on both `lakehouse-store`
    //! (which runs the rotation) and `lakehouse-auth` (which verifies) —
    //! neither of those can test the round trip on its own.
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use lakehouse_auth::Secret;
    use lakehouse_auth::service_token::{create_service_credential, verify_service_token};
    use lakehouse_store::identity;
    use uuid::Uuid;

    /// `bi-dashboard-reader` in `0002_seed_identity.sql`: current, unexpired.
    const SEEDED_IDENTITY: Uuid = Uuid::from_u128(0x4444_4444_4444_4444_8444_0000_0000_0001);

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_rotated_secret_authenticates_and_the_old_one_no_longer_does(
        pool: lakehouse_store::PgPool,
    ) {
        let old = create_service_credential(&pool, SEEDED_IDENTITY)
            .await
            .unwrap();
        assert!(verify_service_token(&pool, &old).await.is_ok());

        let rotated = identity::rotate_service_identity(&pool, SEEDED_IDENTITY, || {
            let (token, token_hash) = lakehouse_auth::service_token::mint_credential();
            (token.expose().to_owned(), token_hash)
        })
        .await
        .unwrap();

        let principal = verify_service_token(&pool, &Secret::new(rotated.secret))
            .await
            .expect("the secret rotation returns must authenticate");
        assert_eq!(principal.display_name, "bi-dashboard-reader");
        assert!(
            verify_service_token(&pool, &old).await.is_err(),
            "the credential rotation replaced must stop authenticating"
        );
    }
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
        use axum::http::{Method, Request};
        use tower::ServiceExt;

        // (path, method) — the rotate route is POST-only, so the GET-by-
        // default request shape the other paths use would hit axum's
        // method-mismatch path and never reach `auth_gate`. POST keeps the
        // path reaching the auth gate, which is what this test is actually
        // asserting against.
        let paths = [
            ("/api/identity/users", Method::GET),
            ("/api/identity/roles", Method::GET),
            ("/api/identity/tenants", Method::GET),
            ("/api/identity/service-identities", Method::GET),
            (
                "/api/identity/service-identities/c4f7a7b2-0000-4000-8000-000000000001/rotate",
                Method::POST,
            ),
        ];
        for (path, method) in paths {
            let app = crate::routes::router(state_without_pool());
            let response = app
                .oneshot(
                    Request::builder()
                        .method(method)
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

    /// The rotate route's audit row is built with a literal `NewAuditEvent`
    /// shape (`action` / `resource_kind` / `resource_id` / `principal_kind` /
    /// `outcome`) that the schema's `audit_event_*_check` constraints accept.
    /// Asserting the literal here is what pins down Hard Requirement 5's
    /// "rotate is audited" property at the field level — the `INSERT`
    /// itself is exercised by the integration tests in
    /// `lakehouse-store/tests/audit.rs`, which use the same `insert` helper
    /// `rotate_service_identity` calls.
    #[test]
    fn rotate_service_identity_audit_event_has_the_expected_fields() {
        use lakehouse_auth::{PermissionSet, PrincipalId};

        let identity_id = uuid::Uuid::from_u128(0xDEAD_BEEF);
        let principal = lakehouse_auth::Principal {
            id: PrincipalId::User(uuid::Uuid::from_u128(0xCAFE_BABE)),
            tenant_ids: Vec::new(),
            display_name: "Rina Wijaya".to_owned(),
            permissions: PermissionSet::parse("identity:write"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        };

        let event = rotate_service_identity_audit_event(&principal, identity_id);
        assert_eq!(event.action, "identity.service_identity.rotate");
        assert_eq!(event.resource_kind.as_deref(), Some("service_identity"));
        // `Uuid::to_string` renders the canonical hyphenated form, which is
        // what `audit_event.resource_id` should carry — same encoding the
        // server-side `Principal::id.uuid().to_string()` produces for
        // `principal_id`.
        assert_eq!(
            event.resource_id.as_deref(),
            Some(identity_id.to_string().as_str())
        );
        assert_eq!(
            event.principal_id.as_deref(),
            Some(principal.id.uuid().to_string().as_str())
        );
        // `provider = "session"` is the authenticator's vocabulary — the
        // audit row must NOT carry it (the audit CHECK only allows
        // `"user" | "service" | "copilot" | "schedule"`). `kind_for_audit`
        // is what maps the principal's variant onto the audit vocabulary.
        assert_eq!(event.principal_kind.as_deref(), Some("user"));
        assert_eq!(event.actor_label.as_deref(), Some("Rina Wijaya"));
        assert_eq!(event.outcome, "executed");
    }
}
