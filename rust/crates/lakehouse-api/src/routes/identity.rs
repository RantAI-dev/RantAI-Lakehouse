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
//!
//! # ADM-3: an invite carries a password
//!
//! `POST /api/identity/users` no longer creates a credential-less
//! directory row: it creates the user AND a `local` credential, in ONE
//! transaction, and returns the temporary password ONCE, on that response.
//! `POST /api/identity/users/{id}/reset-password` does the same for an
//! existing account. Both passwords are generated SERVER-side from the OS
//! random source; neither is ever stored in clear, logged, audited, or
//! listed. See [`create_user`] and [`reset_user_password`].

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use lakehouse_auth::Principal;
use lakehouse_auth::PrincipalId;
use lakehouse_auth::Secret;
use lakehouse_auth::openfga::{LakekeeperAdminClient, TenantWarehouseStorage};
use lakehouse_auth::password;
use lakehouse_auth::session;
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::StoreError;
use lakehouse_store::audit::{self as store_audit, NewAuditEvent};
use lakehouse_store::governance;
use lakehouse_store::identity::{
    self, CreateRoleInput, CreateServiceIdentityInput, CreateTenantInput, InviteUserInput, Role,
    RotateServiceIdentityResponse, ServiceIdentity, ServiceIdentityFilter, Tenant, TenantFilter,
    User, UserFilter,
};
use serde::Deserialize;
use serde::Serialize;
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

/// Password specification for `POST /api/identity/users` (ADM-11 D7).
#[derive(Debug, Deserialize, Clone, Default)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum PasswordInput {
    /// Server generates a temporary password, returned once in the response.
    #[default]
    Generate,
    /// Administrator types the password directly, checked against password rules.
    Set {
        /// Plaintext password chosen by the administrator.
        value: String,
    },
}

fn default_true() -> bool {
    true
}

/// The `POST /api/identity/users` body. Mirrors `InviteUserInput` in
/// `contracts/identity.ts`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteUserBody {
    /// Display name.
    pub name: String,
    /// Unique username (ADM-11 D7).
    #[serde(default)]
    pub username: Option<String>,
    /// Email address (optional, ADM-11 D2.4).
    #[serde(default)]
    pub email: Option<String>,
    /// Role names to grant. Optional in the wire format (defaults to none)
    /// so an invite with no roles doesn't have to send `[]`.
    #[serde(default)]
    pub roles: Vec<String>,
    /// Tenant names to join.
    #[serde(default)]
    pub tenants: Vec<String>,
    /// Password choice: generate or set (ADM-11 D7).
    #[serde(default)]
    pub password: PasswordInput,
    /// Whether the user must change password at first sign-in (ADM-11 D7).
    #[serde(default = "default_true")]
    pub must_change_password: bool,
}

/// Serialize a [`Secret`] as the plain string it wraps.
///
/// ADM-11 review 1 S2 / ADM-3 D1: The ONE place a secret leaves the type that
/// keeps it out of log lines: a [`Secret`]'s own `Debug`/`Display` render a
/// fixed redaction marker, so a response that must carry the value has to unwrap
/// it explicitly, here, in one place. Nothing else in this crate serializes a secret.
fn serialize_secret<S: serde::Serializer>(
    secret: &Secret,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(secret.expose())
}

#[allow(
    clippy::ref_option,
    reason = "serde serialize_with requires &Option<T> signature (ADM-11 review 1 S2)"
)]
fn serialize_optional_secret<S: serde::Serializer>(
    secret: &Option<Secret>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match secret {
        Some(s) => serializer.serialize_str(s.expose()),
        None => serializer.serialize_none(),
    }
}

/// The `POST /api/identity/users` 201 body: the created [`User`] plus the
/// temporary password (if generated).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteUserResponse {
    /// The created user.
    #[serde(flatten)]
    pub user: User,
    /// The generated temporary password, shown once (omitted for set mode).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(serialize_with = "serialize_optional_secret")]
    pub temporary_password: Option<Secret>,
}

/// The `POST /api/identity/users/{id}/reset-password` body: the new
/// temporary password, and nothing else.
///
/// ADM-3 D2: returned exactly once, like the invite's.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetPasswordResponse {
    /// The generated temporary password, shown once.
    #[serde(serialize_with = "serialize_secret")]
    pub temporary_password: Secret,
}

/// `POST /api/identity/users` — invite a user. Returns 201 with the created
/// [`User`] and the temporary password.
///
/// # The temporary password (ADM-3 D1)
///
/// The user, its memberships and a `local` credential are written in ONE
/// transaction, so a user can never be left behind without the credential
/// this response promised. The password itself is generated SERVER-side
/// from the OS random source ([`password::generate_temporary_password`]),
/// never taken from the request: a caller cannot choose a weak password
/// for somebody else, and the server never has to store a plaintext it
/// was given.
///
/// It is returned exactly once, here. It is never written to the database
/// in clear (only the `Argon2id` hash is), never logged, never audited,
/// and never listed by `GET /api/identity/users`. The credential is
/// flagged `must_change_password`, so the invitee signs in and is then
/// forced through the change-password screen (ADM-3 D4).
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
    AuthenticatedPrincipal(principal): AuthenticatedPrincipal,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<InviteUserResponse>)> {
    let body: InviteUserBody = parse_body(&body)?;
    let name = required("name", &body.name)?;
    // ADM-11 D7 / ADM-11 review 1 S1: username is required in the invite body; missing or blank -> 400.
    let raw_username = match body.username.as_deref().map(str::trim) {
        Some(u) if !u.is_empty() => u.to_owned(),
        _ => return Err(ApiError::BadRequest("username is required".to_owned()).into()),
    };
    let raw_email = body.email.as_deref().filter(|e| !e.trim().is_empty());

    let has_email = raw_email.is_some();
    let email_string = raw_email.map(str::to_owned);

    // ADM-3 D1 / ADM-11 D7 (ADM-11 review 1 S2): password choice — generated from the OS random
    // source (wrapped in a Secret so it cannot be logged by accident), or typed by the admin
    // and validated against system password rules.
    let (secret_to_store, temporary_password_to_return, password_mode_str) = match &body.password {
        PasswordInput::Generate => {
            let temp = password::generate_temporary_password();
            (temp.clone(), Some(temp), "generate")
        }
        PasswordInput::Set { value } => {
            password::check_rules(value, Some(&raw_username), raw_email)
                .map_err(|err| ApiError::BadRequest(err.to_string()))?;
            (Secret::new(value.clone()), None, "set")
        }
    };

    let input = InviteUserInput {
        name,
        username: raw_username,
        email: email_string,
        roles: body.roles,
        tenants: body.tenants,
    };
    let pool = pool(&state)?;

    // ADM-3 D1 (ADM-11 review 1 S2): user, memberships and credential in ONE transaction. The
    // store owns the user tables and `lakehouse-auth` owns the credential
    // table, so the transaction is opened HERE and handed to both — see
    // `lakehouse_store::identity::create_user_in_tx` and
    // `lakehouse_auth::password::create_local_identity_in_tx`.
    let mut tx = pool.begin().await.map_err(StoreError::from)?;
    let user_id = identity::create_user_in_tx(&mut tx, &input).await?;
    password::create_local_identity_in_tx(
        &mut tx,
        user_id,
        &secret_to_store,
        body.must_change_password,
    )
    .await?;
    tx.commit().await.map_err(StoreError::from)?;

    let user = identity::get_user(pool, &user_id.to_string()).await?;

    // ADM-3 D5 (ADM-11 review 1 S2): the audit row names the actor and the new user,
    // the password mode, and must-change flag. It never carries the password —
    // not even its length.
    let audit_event = user_invite_audit_event(
        &principal,
        &user,
        &input,
        has_email,
        password_mode_str,
        body.must_change_password,
    );
    if let Err(err) = store_audit::insert(pool, audit_event).await {
        tracing::warn!(
            %err,
            action = "identity.user.invite",
            user_id = %user_id,
            "failed to record user-invite audit event"
        );
    }

    Ok((
        StatusCode::CREATED,
        ApiJson(InviteUserResponse {
            user,
            temporary_password: temporary_password_to_return,
        }),
    ))
}

/// Build the `NewAuditEvent` for an invite — the actor, the new user, and
/// what it was granted.
///
/// ADM-3 D5: `args` carries the email and the role/tenant NAMES the user
/// was given. It never carries the password, and never a hash of it: the
/// audit log is a place an operator reads, and a credential does not
/// belong in it.
fn user_invite_audit_event(
    principal: &Principal,
    user: &User,
    input: &InviteUserInput,
    has_email: bool,
    password_mode: &str,
    must_change_password: bool,
) -> NewAuditEvent {
    NewAuditEvent {
        action: "identity.user.invite".to_owned(),
        resource_kind: Some("user".to_owned()),
        resource_id: Some(user.id.clone()),
        principal_id: Some(principal.id.uuid().to_string()),
        principal_kind: Some(principal.kind_for_audit().to_owned()),
        actor_label: Some(principal.display_name.clone()),
        outcome: "executed".to_owned(),
        args: Some(serde_json::json!({
            "username": user.username,
            "hasEmail": has_email,
            "passwordMode": password_mode,
            "mustChangePassword": must_change_password,
            "roles": input.roles,
            "tenants": input.tenants,
        })),
        ..Default::default()
    }
}

/// `POST /api/identity/users/{id}/reset-password` — give an existing user
/// a new temporary password. Returns 200 with the new password.
///
/// # What it does (ADM-3 D2)
///
/// Sets a NEW server-generated temporary password, flags the credential
/// `must_change_password`, and **ends every live session of that user** —
/// a reset that left the old sessions alive would not have locked the
/// target out of anything. The credential write and the session
/// revocation are ONE transaction.
///
/// The password is returned exactly once, here, and is never stored in
/// clear, logged, audited, or listed.
///
/// # No escalation through a reset (ADM-3 D3)
///
/// Only a caller holding the unrestricted `"*:*"` grant may reset the
/// password of a user who holds `"*:*"` — the same rule
/// `add_user_to_tenant` follows ([`crate::routes::catalog::is_unrestricted`]).
/// Anyone else gets the SAME 404 an unknown user gets, so a probe learns
/// nothing about which accounts exist or what they hold.
///
/// A caller may not reset their OWN password here: that is what
/// `POST /api/auth/change-password` is for, and it verifies the old
/// password instead of replacing it behind the holder's back.
///
/// # Security
///
/// Requires `identity:write` — see the module doc comment.
///
/// # Errors
///
/// 400 on a `{id}` that is not a UUID, or on a self-reset; 404 for an
/// unknown user and for a target the caller may not reset (one body for
/// both); 503 if no pool is configured; 500 (classified) on a storage
/// failure. One `identity.user.password_reset` audit event is written on
/// success.
pub async fn reset_user_password(
    State(state): State<AppState>,
    AuthenticatedPrincipal(principal): AuthenticatedPrincipal,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<ResetPasswordResponse>> {
    let user_id = Uuid::parse_str(&id)
        .map_err(|_| ApiError::BadRequest("user id must be a UUID".to_owned()))?;
    let pool = pool(&state)?;

    // ADM-3 D3: a caller may not reset their own password here. The
    // caller is the target, so the answer is a 400 pointing at the route
    // that does belong here — not a 404 (the target certainly exists)
    // and not a 403 (the caller holds `identity:write`).
    if let PrincipalId::User(caller_id) = principal.id
        && caller_id == user_id
    {
        return Err(ApiError::BadRequest(
            "use POST /api/auth/change-password to change your own password".to_owned(),
        )
        .into());
    }

    // An unknown user is a 404, before anything is written.
    if !identity::user_exists(pool, user_id).await? {
        return Err(ApiError::NotFound("record not found".to_owned()).into());
    }

    // The target's EFFECTIVE grants, read through the same repository
    // function every authenticated request's principal is built from — so
    // the escalation rule below judges the grants the target really has,
    // not a re-implementation of them.
    //
    // ADM-4 D4 changed WHICH reader this is: the principal loader now
    // refuses an INACTIVE account (that is the point of D4 — an inactive
    // account has no access on any path), and a password reset is not
    // about access. The status-agnostic reader asks the question this
    // route actually asks — "does this ACCOUNT hold the unrestricted
    // grant?" — and answers it for a deactivated target too, so
    // resetting the password of a deactivated user still works (and
    // leaves them deactivated).
    let target_holds_unrestricted =
        lakehouse_auth::repository::user_holds_unrestricted(pool, user_id).await?;

    // ADM-3 D3: only an unrestricted caller may reset an unrestricted
    // user. The refusal is the SAME 404 an unknown user gets, body for
    // body, so a probe cannot tell the two apart.
    if target_holds_unrestricted && !crate::routes::catalog::is_unrestricted(&principal) {
        return Err(ApiError::NotFound("record not found".to_owned()).into());
    }

    let temporary_password = password::generate_temporary_password();

    // ADM-3 D2: the new credential and the revocation of the target's
    // sessions are one unit of work.
    let mut tx = pool.begin().await.map_err(StoreError::from)?;
    password::set_temporary_password_in_tx(&mut tx, user_id, &temporary_password).await?;
    session::revoke_all_sessions_for_user_in_tx(&mut tx, user_id).await?;
    tx.commit().await.map_err(StoreError::from)?;

    // ADM-3 D5: the audit row names the actor and the target user. Never
    // the password.
    let audit_event = user_password_reset_audit_event(&principal, user_id);
    if let Err(err) = store_audit::insert(pool, audit_event).await {
        tracing::warn!(
            %err,
            action = "identity.user.password_reset",
            user_id = %user_id,
            "failed to record password-reset audit event"
        );
    }

    Ok(ApiJson(ResetPasswordResponse { temporary_password }))
}

/// Build the `NewAuditEvent` for a password reset — the actor and the
/// target user, and nothing else.
///
/// ADM-3 D5: no password, and no hash of one. `resource_id` names the
/// user whose credential changed; `principal_id` names who changed it.
fn user_password_reset_audit_event(principal: &Principal, user_id: Uuid) -> NewAuditEvent {
    NewAuditEvent {
        action: "identity.user.password_reset".to_owned(),
        resource_kind: Some("user".to_owned()),
        resource_id: Some(user_id.to_string()),
        principal_id: Some(principal.id.uuid().to_string()),
        principal_kind: Some(principal.kind_for_audit().to_owned()),
        actor_label: Some(principal.display_name.clone()),
        outcome: "executed".to_owned(),
        args: Some(serde_json::json!({ "targetUserId": user_id.to_string() })),
        ..Default::default()
    }
}

/// `PUT /api/identity/users/{id}/tenants/{tenant_id}` — add an existing
/// user to a tenant. No body. Returns 200 with the updated [`User`], whose
/// `tenants` now lists the tenant; adding a member again is the same 200
/// and changes nothing.
///
/// Before this route a user could be given a tenant only at invite time
/// (`POST /api/identity/users`), so an account created without one (the
/// bootstrap admin) could never get one through the API.
///
/// # Who may add to which tenant
///
/// The caller must hold the unrestricted `*:*` grant
/// ([`crate::routes::catalog::is_unrestricted`]) or already be a member of
/// the tenant. Anyone else gets the same 404 as for a tenant that does not
/// exist, so the answer says nothing about which tenants exist (the rule
/// `tenant_scope::resolve` follows). `POST /api/identity/users` is looser:
/// it accepts any tenant name from any `identity:write` caller. This route
/// is not loosened to match.
///
/// # Errors
///
/// 400 on a `{id}` or `{tenant_id}` that is not a UUID; 404 for a tenant
/// that does not exist or that the caller may not add to (one body for
/// both) and for a user that does not exist; 503 if no pool is
/// configured; 500 (classified) on a storage failure. One
/// `identity.user.tenant.add` audit event is written when a row was added,
/// none when the membership already existed.
pub async fn add_user_to_tenant(
    State(state): State<AppState>,
    AuthenticatedPrincipal(principal): AuthenticatedPrincipal,
    Path((id, tenant_id)): Path<(String, String)>,
) -> ApiResult<ApiJson<User>> {
    let user_id = Uuid::parse_str(&id)
        .map_err(|_| ApiError::BadRequest("user id must be a UUID".to_owned()))?;
    let tenant_id = Uuid::parse_str(&tenant_id)
        .map_err(|_| ApiError::BadRequest("tenant id must be a UUID".to_owned()))?;
    let pool = pool(&state)?;

    // The caller's own right to the tenant is checked before anything is
    // read, and fails with the unknown-tenant answer below.
    let tenant_not_found = || ApiError::NotFound("tenant not found".to_owned());
    if !crate::routes::catalog::is_unrestricted(&principal) && !principal.in_tenant(tenant_id) {
        return Err(tenant_not_found().into());
    }
    if !identity::tenant_exists(pool, tenant_id).await? {
        return Err(tenant_not_found().into());
    }
    // `get_user` answers `StoreError::NotFound` (404) for an unknown user.
    identity::get_user(pool, &user_id.to_string()).await?;

    let added = identity::add_user_to_tenant(pool, user_id, tenant_id).await?;
    if added {
        let audit_event = user_tenant_add_audit_event(&principal, user_id, tenant_id);
        if let Err(err) = store_audit::insert(pool, audit_event).await {
            tracing::warn!(
                %err,
                action = "identity.user.tenant.add",
                user_id = %user_id,
                tenant_id = %tenant_id,
                "failed to record user-tenant-add audit event"
            );
        }
    }

    Ok(ApiJson(
        identity::get_user(pool, &user_id.to_string()).await?,
    ))
}

/// Build the `NewAuditEvent` for a membership that was added — the same
/// shape as [`tenant_create_audit_event`]. The resource is the user whose
/// access changed; the tenant id is in `args`.
fn user_tenant_add_audit_event(
    principal: &Principal,
    user_id: Uuid,
    tenant_id: Uuid,
) -> NewAuditEvent {
    NewAuditEvent {
        action: "identity.user.tenant.add".to_owned(),
        resource_kind: Some("user".to_owned()),
        resource_id: Some(user_id.to_string()),
        principal_id: Some(principal.id.uuid().to_string()),
        principal_kind: Some(principal.kind_for_audit().to_owned()),
        actor_label: Some(principal.display_name.clone()),
        outcome: "executed".to_owned(),
        args: Some(serde_json::json!({ "tenantId": tenant_id.to_string() })),
        ..Default::default()
    }
}

/// `DELETE /api/identity/users/{id}/tenants/{tenant_id}` — take an
/// existing user out of a tenant. No body. Returns 200 with the updated
/// [`User`], whose `tenants` no longer lists the tenant; removing a
/// member who is not a member is the same 200 and changes nothing.
///
/// The counterpart of [`add_user_to_tenant`], under the SAME "who may"
/// rule: the caller must hold the unrestricted `*:*` grant
/// ([`crate::routes::catalog::is_unrestricted`]) or already be a member
/// of the tenant. Anyone else gets the same 404 as for a tenant that does
/// not exist, so the answer says nothing about which tenants exist.
///
/// # Errors
///
/// 400 on a `{id}` or `{tenant_id}` that is not a UUID; 404 for a tenant
/// that does not exist or that the caller may not change (one body for
/// both) and for a user that does not exist; 503 if no pool is
/// configured; 500 (classified) on a storage failure. One
/// `identity.user.tenant.remove` audit event is written when a membership
/// was removed, none when the user was not in the tenant.
pub async fn remove_user_from_tenant(
    State(state): State<AppState>,
    AuthenticatedPrincipal(principal): AuthenticatedPrincipal,
    Path((id, tenant_id)): Path<(String, String)>,
) -> ApiResult<ApiJson<User>> {
    let user_id = Uuid::parse_str(&id)
        .map_err(|_| ApiError::BadRequest("user id must be a UUID".to_owned()))?;
    let tenant_id = Uuid::parse_str(&tenant_id)
        .map_err(|_| ApiError::BadRequest("tenant id must be a UUID".to_owned()))?;
    let pool = pool(&state)?;

    // The caller's own right to the tenant is checked before anything is
    // read, and fails with the unknown-tenant answer below.
    let tenant_not_found = || ApiError::NotFound("tenant not found".to_owned());
    if !crate::routes::catalog::is_unrestricted(&principal) && !principal.in_tenant(tenant_id) {
        return Err(tenant_not_found().into());
    }
    if !identity::tenant_exists(pool, tenant_id).await? {
        return Err(tenant_not_found().into());
    }
    // `get_user` answers `StoreError::NotFound` (404) for an unknown user.
    identity::get_user(pool, &user_id.to_string()).await?;

    let mut tx = pool.begin().await.map_err(StoreError::from)?;
    let removed = identity::remove_user_from_tenant_in_tx(&mut tx, user_id, tenant_id).await?;
    tx.commit().await.map_err(StoreError::from)?;

    if removed {
        let audit_event = user_tenant_remove_audit_event(&principal, user_id, tenant_id);
        if let Err(err) = store_audit::insert(pool, audit_event).await {
            tracing::warn!(
                %err,
                action = "identity.user.tenant.remove",
                user_id = %user_id,
                tenant_id = %tenant_id,
                "failed to record user-tenant-remove audit event"
            );
        }
    }

    Ok(ApiJson(
        identity::get_user(pool, &user_id.to_string()).await?,
    ))
}

/// Build the `NewAuditEvent` for a membership that was removed — the
/// mirror of [`user_tenant_add_audit_event`].
fn user_tenant_remove_audit_event(
    principal: &Principal,
    user_id: Uuid,
    tenant_id: Uuid,
) -> NewAuditEvent {
    NewAuditEvent {
        action: "identity.user.tenant.remove".to_owned(),
        resource_kind: Some("user".to_owned()),
        resource_id: Some(user_id.to_string()),
        principal_id: Some(principal.id.uuid().to_string()),
        principal_kind: Some(principal.kind_for_audit().to_owned()),
        actor_label: Some(principal.display_name.clone()),
        outcome: "executed".to_owned(),
        args: Some(serde_json::json!({ "tenantId": tenant_id.to_string() })),
        ..Default::default()
    }
}

/// The `PUT /api/identity/users/{id}/roles` body: the user's COMPLETE
/// local role set, by name.
///
/// ADM-4 D1: the request carries the whole set, not a delta — what the
/// user holds after the call is exactly what this names.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetUserRolesBody {
    /// Role names. An empty list leaves the user with no local role at
    /// all.
    #[serde(default)]
    roles: Vec<String>,
}

/// `PUT /api/identity/users/{id}/roles` — replace a user's local role
/// set. Returns 200 with the updated [`User`].
///
/// # What it replaces, and what it does not (ADM-4 D1)
///
/// The request names the user's COMPLETE local role set, and the user's
/// local role set after the call is exactly that. Roles mapped from an
/// identity provider's groups (`OIDC_ROLE_MAP`) are NOT stored and NOT
/// touched: they are re-resolved on every request from the provider's
/// claims, so there is nothing here to replace. The user keeps the union
/// of the two, exactly as before.
///
/// # Who may change whose roles (ADM-4 D5)
///
/// * Only an unrestricted (`*:*`) caller may change the roles of a user
///   who holds `*:*`, or grant a role that holds `*:*`. Anyone else gets
///   the SAME 404 an unknown user gets, body for body, so a probe learns
///   nothing about which accounts exist or what they hold (ADM-3 D3's
///   rule).
/// * A caller may not remove their OWN `*:*` — a 400, because the caller
///   is the target and the answer is about the request, not about a
///   missing record.
/// * A change that would leave NO active user holding `*:*` is refused
///   with a 409, in the same transaction as the change: a deployment with
///   no administrator is not a state an administrator can be talked into.
///
/// # Errors
///
/// 400 on a `{id}` that is not a UUID, on an unknown role name, and on a
/// caller removing their own `*:*`; 404 for an unknown user and for a
/// target the caller may not change (one body for both); 409 when the
/// change would leave no active unrestricted user; 503 if no pool is
/// configured; 500 (classified) on a storage failure. One
/// `identity.user.roles.set` audit event is written on success, naming
/// the old and the new role names.
pub async fn set_user_roles(
    State(state): State<AppState>,
    AuthenticatedPrincipal(principal): AuthenticatedPrincipal,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<ApiJson<User>> {
    let user_id = Uuid::parse_str(&id)
        .map_err(|_| ApiError::BadRequest("user id must be a UUID".to_owned()))?;
    let body: SetUserRolesBody = parse_body(&body)?;
    let pool = pool(&state)?;

    // The names as they will be stored: trimmed, and without the blanks a
    // comma-separated list in a browser form leaves behind. Duplicates
    // collapse — the role set is a set.
    let mut requested: Vec<String> = Vec::new();
    for raw in &body.roles {
        let name = raw.trim();
        if name.is_empty() {
            continue;
        }
        if !requested.iter().any(|existing| existing == name) {
            requested.push(name.to_owned());
        }
    }

    // ADM-4 D5: only an unrestricted caller may change the roles of an
    // unrestricted user. The refusal is the SAME 404 an unknown user
    // gets, body for body, so a probe cannot tell the two apart.
    //
    // The answer is asked ONCE and kept: the same fact decides the
    // self-lockout rule further down, and two reads could disagree (a
    // concurrent change in between), which would make the two rules
    // disagree with each other.
    let target_holds_unrestricted =
        lakehouse_auth::repository::user_holds_unrestricted(pool, user_id).await?;
    if target_holds_unrestricted && !crate::routes::catalog::is_unrestricted(&principal) {
        return Err(ApiError::NotFound("record not found".to_owned()).into());
    }
    // An unknown user is the same 404.
    identity::get_user(pool, &user_id.to_string()).await?;

    let mut tx = pool.begin().await.map_err(StoreError::from)?;

    // The roles the user holds now — the audit's "before" half.
    let old_roles = identity::user_role_names_in_tx(&mut tx, user_id).await?;

    // The roles the request names, and what they grant.
    let resolved = identity::resolve_roles_in_tx(&mut tx, &requested).await?;
    let grants_unrestricted = resolved
        .iter()
        .any(|role| lakehouse_auth::PermissionSet::parse(&role.permissions).grants_unrestricted());

    // ADM-4 D5: only an unrestricted caller may GRANT a role that holds
    // `*:*`. The refusal is the same 404 as above, and it comes BEFORE
    // the unknown-name answer below: a caller who may not grant the
    // unrestricted role learns nothing about which role names exist.
    if grants_unrestricted && !crate::routes::catalog::is_unrestricted(&principal) {
        return Err(ApiError::NotFound("record not found".to_owned()).into());
    }

    // ADM-4 D1: an unknown role name is a 400.
    if resolved.len() != requested.len() {
        let unknown = requested
            .iter()
            .find(|name| !resolved.iter().any(|role| role.name == name.as_str()))
            .cloned()
            .unwrap_or_default();
        return Err(ApiError::BadRequest(format!(
            "unknown role name: {}",
            unknown.chars().take(64).collect::<String>()
        ))
        .into());
    }

    // ADM-4 D5: a caller may not remove their OWN `*:*`. The caller is
    // the target here, so the answer is a 400 about the request — not a
    // 404 (the target certainly exists) and not a 403 (the caller holds
    // `identity:write`).
    //
    // The check comes AFTER the "would the deployment lose its
    // administrator?" one below, because a caller who IS the last
    // administrator is refused by THAT rule (a 409, naming the deployment
    // as the reason) rather than by this one: it is the stronger,
    // deployment-wide answer, and it is the one an operator needs to see.
    let role_ids: Vec<Uuid> = resolved.iter().map(|role| role.id).collect();
    identity::set_user_roles_in_tx(&mut tx, user_id, &role_ids).await?;

    // ADM-4 D5: the change and the count that guards it are one unit of
    // work. The count is read AFTER the change, inside the same
    // transaction: an empty answer rolls the change back.
    let administrators =
        lakehouse_auth::repository::active_unrestricted_user_ids_in_tx(&mut tx).await?;
    if administrators.is_empty() {
        return Err(ApiError::Conflict(
            "the change would leave no active administrator".to_owned(),
        )
        .into());
    }

    if let PrincipalId::User(caller_id) = principal.id
        && caller_id == user_id
        && target_holds_unrestricted
        && !grants_unrestricted
    {
        return Err(ApiError::BadRequest(
            "you may not remove your own unrestricted grant".to_owned(),
        )
        .into());
    }

    tx.commit().await.map_err(StoreError::from)?;

    let user = identity::get_user(pool, &user_id.to_string()).await?;

    // ADM-4 D6: the audit names the old AND the new role names, so an
    // operator can see what the change took away.
    let audit_event = user_roles_set_audit_event(&principal, &user, &old_roles, &requested);
    if let Err(err) = store_audit::insert(pool, audit_event).await {
        tracing::warn!(
            %err,
            action = "identity.user.roles.set",
            user_id = %user_id,
            "failed to record user-roles-set audit event"
        );
    }

    Ok(ApiJson(user))
}

/// Build the `NewAuditEvent` for a role change — the actor, the target,
/// and the role names before and after.
fn user_roles_set_audit_event(
    principal: &Principal,
    user: &User,
    old_roles: &[String],
    new_roles: &[String],
) -> NewAuditEvent {
    NewAuditEvent {
        action: "identity.user.roles.set".to_owned(),
        resource_kind: Some("user".to_owned()),
        resource_id: Some(user.id.clone()),
        principal_id: Some(principal.id.uuid().to_string()),
        principal_kind: Some(principal.kind_for_audit().to_owned()),
        actor_label: Some(principal.display_name.clone()),
        outcome: "executed".to_owned(),
        args: Some(serde_json::json!({
            "oldRoles": old_roles,
            "newRoles": new_roles,
        })),
        ..Default::default()
    }
}

/// `POST /api/identity/users/{id}/deactivate` — switch a user off.
/// Returns 200 with the updated [`User`], whose `status` is now
/// `"inactive"`.
///
/// # What it does (ADM-4 D3)
///
/// Sets `app_user.status = 'inactive'` and ends **every session that
/// user holds**, in ONE transaction. The user's data, dashboards and
/// audit history stay exactly where they are: a deactivated person is
/// not a deleted one.
///
/// The user's API tokens are revoked too — there are none: a personal
/// API token is not a thing in this schema. `service_credential` (the
/// token store) belongs to `service_identity`, and a service identity is
/// a separate principal that no user-route touches.
///
/// # Who may (ADM-4 D5)
///
/// Only an unrestricted (`*:*`) caller may switch an unrestricted user
/// off; anyone else gets the same 404 an unknown user gets, body for
/// body. A caller may not switch THEMSELVES off (a 400: the caller is
/// the target, so the answer is about the request, not about a missing
/// record). A change that would leave no active user holding `*:*` is
/// refused with a 409, in the same transaction as the change.
///
/// # Errors
///
/// 400 on a `{id}` that is not a UUID and on a self-deactivation; 404 for
/// an unknown user and for a target the caller may not change (one body
/// for both); 409 when the change would leave no active administrator;
/// 503 if no pool is configured; 500 (classified) on a storage failure.
/// One `identity.user.deactivate` audit event is written on success.
pub async fn deactivate_user(
    State(state): State<AppState>,
    AuthenticatedPrincipal(principal): AuthenticatedPrincipal,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<User>> {
    change_user_status(
        &state,
        &principal,
        &id,
        "identity.user.deactivate",
        "inactive",
    )
    .await
}

/// `POST /api/identity/users/{id}/reactivate` — switch a user back on.
/// Returns 200 with the updated [`User`], whose `status` is now
/// `"active"`.
///
/// The mirror of [`deactivate_user`], with the same guards: only an
/// unrestricted caller may switch an unrestricted user back on (the same
/// 404 an unknown user gets), and the change may not leave the
/// deployment without an active administrator — which a reactivation
/// never can, since it only ever adds one.
///
/// # Errors
///
/// 400 on a `{id}` that is not a UUID; 404 for an unknown user and for a
/// target the caller may not change (one body for both); 409 when the
/// change would leave no active administrator; 503 if no pool is
/// configured; 500 (classified) on a storage failure. One
/// `identity.user.reactivate` audit event is written on success.
pub async fn reactivate_user(
    State(state): State<AppState>,
    AuthenticatedPrincipal(principal): AuthenticatedPrincipal,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<User>> {
    change_user_status(
        &state,
        &principal,
        &id,
        "identity.user.reactivate",
        "active",
    )
    .await
}

/// Switch a user off (`"inactive"`) or back on (`"active"`): the status
/// change, the end of the target's live sessions (when switching them
/// off), the check that the deployment still has an administrator, and
/// the audit row — one unit of work.
///
/// ADM-4 D3: deactivating ends EVERY session the user holds, in the same
/// transaction as the status change. A deactivation that left the old
/// sessions alive would not have switched the person off at all.
///
/// ADM-4 D5: only an unrestricted caller may switch an unrestricted user
/// off or on — anyone else gets the same 404 an unknown user gets. A
/// caller may not switch themselves off. A change that would leave no
/// active user holding `*:*` is refused with a 409, in the same
/// transaction as the change.
async fn change_user_status(
    state: &AppState,
    principal: &Principal,
    id: &str,
    action: &str,
    status: &str,
) -> ApiResult<ApiJson<User>> {
    let user_id = Uuid::parse_str(id)
        .map_err(|_| ApiError::BadRequest("user id must be a UUID".to_owned()))?;
    let pool = pool(state)?;

    // ADM-4 D5: only an unrestricted caller may switch an unrestricted
    // user off or on. The refusal is the SAME 404 an unknown user gets,
    // body for body, so a probe cannot tell the two apart.
    if lakehouse_auth::repository::user_holds_unrestricted(pool, user_id).await?
        && !crate::routes::catalog::is_unrestricted(principal)
    {
        return Err(ApiError::NotFound("record not found".to_owned()).into());
    }
    // An unknown user is the same 404.
    if !identity::user_exists(pool, user_id).await? {
        return Err(ApiError::NotFound("record not found".to_owned()).into());
    }

    let deactivating = status == "inactive";

    let mut tx = pool.begin().await.map_err(StoreError::from)?;
    identity::set_user_status_in_tx(&mut tx, user_id, status).await?;
    if deactivating {
        // ADM-4 D3: every session the target holds ends NOW, in the same
        // transaction as the status change.
        session::revoke_all_sessions_for_user_in_tx(&mut tx, user_id).await?;
    }

    // ADM-4 D5: the change and the count that guards it are one unit of
    // work. The count is read AFTER the change, inside the same
    // transaction: an empty answer rolls the change back.
    let administrators =
        lakehouse_auth::repository::active_unrestricted_user_ids_in_tx(&mut tx).await?;
    if administrators.is_empty() {
        return Err(ApiError::Conflict(
            "the change would leave no active administrator".to_owned(),
        )
        .into());
    }

    // ADM-4 D5: a caller may not deactivate themselves. The caller is
    // the target here, so the answer is a 400 about the request — not a
    // 404 (the target certainly exists) and not a 403 (the caller holds
    // `identity:write`).
    //
    // It comes after the "would the deployment lose its administrator?"
    // check above, because a caller who IS the last administrator is
    // refused by THAT rule (a 409, naming the deployment as the reason)
    // rather than by this one.
    if deactivating
        && let PrincipalId::User(caller_id) = principal.id
        && caller_id == user_id
    {
        return Err(
            ApiError::BadRequest("you may not deactivate your own account".to_owned()).into(),
        );
    }

    tx.commit().await.map_err(StoreError::from)?;

    let user = identity::get_user(pool, &user_id.to_string()).await?;

    let audit_event = user_status_audit_event(principal, &user, action, status);
    if let Err(err) = store_audit::insert(pool, audit_event).await {
        tracing::warn!(
            %err,
            action = action,
            user_id = %user_id,
            "failed to record user-status audit event"
        );
    }

    Ok(ApiJson(user))
}

/// Build the `NewAuditEvent` for a status change — the actor, the
/// target, and the status it moved to.
fn user_status_audit_event(
    principal: &Principal,
    user: &User,
    action: &str,
    status: &str,
) -> NewAuditEvent {
    NewAuditEvent {
        action: action.to_owned(),
        resource_kind: Some("user".to_owned()),
        resource_id: Some(user.id.clone()),
        principal_id: Some(principal.id.uuid().to_string()),
        principal_kind: Some(principal.kind_for_audit().to_owned()),
        actor_label: Some(principal.display_name.clone()),
        outcome: "executed".to_owned(),
        args: Some(serde_json::json!({ "status": status })),
        ..Default::default()
    }
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

/// The `PUT /api/identity/roles/{id}` body. Mirrors `UpdateRoleInput`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRoleBody {
    /// Role name; must not collide with another role's.
    name: String,
    /// Free-text permission list.
    #[serde(default)]
    permissions: String,
    /// Human-readable description.
    #[serde(default)]
    description: String,
}

/// `PUT /api/identity/roles/{id}` — rewrite a role's name, permissions and
/// description (`WEB-3` S3a-D2). Audited as `identity.role.update`,
/// naming the old and the new bundle.
///
/// # Refusals
///
/// A rebuilt bundle that keeps the name is unaffected. A rename is refused
/// with 409 while any access rule — any status, because a draft becomes
/// live later — names the OLD name among the roles its structured
/// [`PolicyCondition`](crate::policy_engine::PolicyCondition) applies to,
/// and then nothing is written. Rules are matched against a principal's
/// role NAMES, so a rename would silently stop every mask and row filter
/// those rules carry from reaching the people who hold this role: data the
/// role must not see would become visible (`WEB-3` review R2).
///
/// # Security
///
/// Requires `identity:write`, the permission `POST /api/identity/roles`
/// uses — this edits the same kind of permission bundle that route mints.
///
/// # Errors
///
/// 400 on a malformed body or a blank name; 404 when no role has that id;
/// 409 when another role has the name, or when an access rule names this
/// one; 503/500 as above.
pub async fn update_role(
    State(state): State<AppState>,
    AuthenticatedPrincipal(principal): AuthenticatedPrincipal,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<ApiJson<Role>> {
    let body: UpdateRoleBody = parse_body(&body)?;
    let input = identity::UpdateRoleInput {
        name: required("name", &body.name)?,
        permissions: body.permissions,
        description: body.description,
    };
    let pool = pool(&state)?;
    let before = identity::get_role(pool, &id).await?;
    // WEB-3 review R2: a rename would take every mask and row filter the
    // rules naming this role carry away from its holders, because rules
    // match on names. Refuse before anything is written, listing the rules
    // from the stored rows.
    if input.name != before.name {
        let naming = policies_naming_role(pool, &before.name).await?;
        if !naming.is_empty() {
            return Err(ApiError::Conflict(named_by_rules_sentence(&naming)).into());
        }
    }
    let role = identity::update_role(pool, &id, &input).await?;
    let audit_event = role_update_audit_event(&principal, &before, &role);
    if let Err(err) = store_audit::insert(pool, audit_event).await {
        tracing::warn!(
            %err,
            action = "identity.role.update",
            role_id = %role.id,
            "failed to record role-update audit event"
        );
    }
    Ok(ApiJson(role))
}

/// `DELETE /api/identity/roles/{id}` — remove a role nobody holds
/// (`WEB-3` S3a-D2). Returns 204. Audited as `identity.role.delete`.
///
/// # Refusals
///
/// A built-in role (one [`identity::BUILT_IN_ROLE_IDS`] names — the product's
/// seeded bundles) is refused with 409 "A built-in role cannot be deleted.",
/// before the members check: it stays a built-in role however many people
/// hold it. A role people still hold is refused with 409 "This role is
/// still given to N people. Take it away from them first." — `N` counted
/// here, never read out of a foreign-key violation's own text (principle 4).
///
/// # Security
///
/// Requires `identity:write`, the permission `POST /api/identity/roles`
/// uses.
///
/// # Errors
///
/// 404 when no role has that id; 409 as above; 503/500 as above.
pub async fn delete_role(
    State(state): State<AppState>,
    AuthenticatedPrincipal(principal): AuthenticatedPrincipal,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let pool = pool(&state)?;
    let role = identity::get_role(pool, &id).await?;
    if identity::is_built_in_role(&role.id) {
        return Err(ApiError::Conflict("A built-in role cannot be deleted.".to_owned()).into());
    }
    if role.members > 0 {
        return Err(ApiError::Conflict(still_given_sentence(role.members)).into());
    }
    match identity::delete_role(pool, &id).await {
        Ok(()) => {}
        // A user was given the role between the count and the delete: the
        // row is held after all. The fixed sentence, not the constraint's
        // text (principle 4).
        Err(StoreError::ForeignKeyViolation) => {
            return Err(ApiError::Conflict(still_given_sentence(role.members.max(1))).into());
        }
        Err(err) => return Err(err.into()),
    }
    let audit_event = role_audit_event(
        &principal,
        "identity.role.delete",
        &role.id,
        serde_json::json!({ "name": role.name, "permissions": role.permissions }),
    );
    if let Err(err) = store_audit::insert(pool, audit_event).await {
        tracing::warn!(
            %err,
            action = "identity.role.delete",
            role_id = %role.id,
            "failed to record role-delete audit event"
        );
    }
    Ok(StatusCode::NO_CONTENT)
}

/// The 409 sentence for a role people still hold (`WEB-3` S3a-D2, fixed by
/// the plan).
fn still_given_sentence(members: i64) -> String {
    format!("This role is still given to {members} people. Take it away from them first.")
}

/// Every access rule whose structured `conditions` name `role`, in the
/// store's order (newest first). `policy_engine::condition_names_role` is
/// the read-time rule, so this asks the engine's own question; every
/// status is considered, because a draft is enforced the moment someone
/// makes it ready (`WEB-3` review R2).
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the read fails.
async fn policies_naming_role(
    pool: &PgPool,
    role: &str,
) -> Result<Vec<governance::Policy>, StoreError> {
    let policies = governance::list_policies(pool).await?;
    Ok(policies
        .into_iter()
        .filter(|policy| {
            crate::policy_engine::condition_names_role(policy.conditions.as_deref(), role)
        })
        .collect())
}

/// The 409 sentence for a rename an access rule still names (`WEB-3`
/// review R2, fixed by the plan): how many rules name the role, and the
/// first three by name — read from the stored rows, never from an
/// upstream error's own text (principle 4).
fn named_by_rules_sentence(rules: &[governance::Policy]) -> String {
    let first_three = rules
        .iter()
        .take(3)
        .map(|rule| rule.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "This role is named by {} access rules ({first_three}…). Change those rules first, or \
         keep the role's name.",
        rules.len()
    )
}

/// The audit row for a role rewrite: the actor, the role, and the bundle
/// before and after, so an operator can see what the change took away.
fn role_update_audit_event(principal: &Principal, before: &Role, after: &Role) -> NewAuditEvent {
    role_audit_event(
        principal,
        "identity.role.update",
        &after.id,
        serde_json::json!({
            "oldName": before.name,
            "newName": after.name,
            "oldPermissions": before.permissions,
            "newPermissions": after.permissions,
        }),
    )
}

/// The audit row for a role change the console makes (`WEB-3` S3a-D2): who
/// acted, what they did, which role, and the arguments that say what
/// changed.
fn role_audit_event(
    principal: &Principal,
    action: &str,
    role_id: &str,
    args: serde_json::Value,
) -> NewAuditEvent {
    NewAuditEvent {
        action: action.to_owned(),
        resource_kind: Some("role".to_owned()),
        resource_id: Some(role_id.to_owned()),
        principal_id: Some(principal.id.uuid().to_string()),
        principal_kind: Some(principal.kind_for_audit().to_owned()),
        actor_label: Some(principal.display_name.clone()),
        outcome: "executed".to_owned(),
        args: Some(args),
        ..Default::default()
    }
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
/// # Resumable provisioning
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
/// than rejecting with 409 — see [`create_tenant`]'s doc comment.
/// Deliberately excludes `complete` and `not_applicable`: neither is a
/// step to resume from, and treating either as resumable would silently
/// re-run provisioning against a tenant that is already done.
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
    // mid-provisioning. A 409 is reserved for a
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

    // audit_event row mirrors 0024_audit_event.sql's schema — `outcome = "executed"`
    // per `audit_event_outcome_check` (0024_audit_event.sql). Written
    // BEFORE the `lakekeeper_admin` check so the audit fires whether or
    // not provisioning is reachable on this deployment — the tenant row
    // already exists in either branch (resume or fresh create), so an
    // operator's history is honest about what happened. Best-effort: a
    // failed audit write is logged and swallowed, never propagated, so
    // the response code on the wire is always the underlying operation's,
    // not the audit's.
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

/// A tenant warehouse's `key-prefix` inside the dedicated tenant bucket
/// (`TENANT_WAREHOUSE_S3_BUCKET`, see [`tenant_warehouse_storage`]). It
/// is the slug alone: the bucket already belongs to tenant warehouses, so
/// a bucket name inside the prefix would only name a different bucket in
/// the object path. The trailing `/` keeps a slug that is a string prefix
/// of another (`acme` / `acme-eu`) from overlapping it in Lakekeeper.
fn tenant_key_prefix(slug: &str) -> String {
    format!("{slug}/")
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
/// [`crate::error::tenant_warehouse_storage_not_configured`] (503) if this
/// deployment has not set the dedicated `TENANT_WAREHOUSE_S3_*` settings
/// [`tenant_warehouse_storage`] requires, or
/// [`crate::error::tenant_warehouse_bucket_overlaps_default`] (503) if
/// `TENANT_WAREHOUSE_S3_BUCKET` equals `LAKEHOUSE_WAREHOUSE_BUCKET` —
/// both checked before ever attempting the Lakekeeper call, an honest
/// refusal rather than a request Lakekeeper would reject anyway.
/// [`crate::error::provisioning_unavailable`] (503) if
/// a Lakekeeper call itself fails — never `err.to_string()`, per
/// AGENTS.md's "upstream error text never reaches a response". A
/// [`lakehouse_core::StoreError`] from a checkpoint write propagates as-is
/// via `?`.
async fn provision_tenant(
    pool: &PgPool,
    admin: &LakekeeperAdminClient,
    mut tenant: Tenant,
    config: &Config,
) -> ApiResult<Tenant> {
    let warehouse_name = format!("tenant-{}", tenant.slug);
    let prefix = tenant_key_prefix(&tenant.slug);

    if tenant.provisioning_status == "pending" {
        let storage = tenant_warehouse_storage(config)?;
        let warehouse_id = admin
            .ensure_warehouse(&warehouse_name, &prefix, &storage)
            .await
            .map_err(|err| crate::error::provisioning_unavailable(&err))?;
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
            .map_err(|err| crate::error::provisioning_unavailable(&err))?;
        tenant =
            identity::update_tenant_provisioning_status(pool, &tenant.id, "grants_ready", None)
                .await?;
    }
    // Registry namespace: this state machine reserves
    // `namespace_ready`/`complete` for creating this tenant's Iceberg
    // namespace — but neither status is written
    // yet, because no namespace is created yet. `lakehouse-iceberg`'s only
    // namespace-create surface (`ensure_bronze_namespace`/
    // `ensure_gold_namespace`) is hardcoded to this deployment's one shared
    // catalog, not parameterized by a tenant warehouse, so calling it would
    // create a namespace on the WRONG catalog; inventing a second,
    // tenant-scoped Iceberg client here is out of scope for this route.
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

/// Build the [`TenantWarehouseStorage`] `LakekeeperAdminClient::ensure_warehouse`
/// needs from this deployment's dedicated `TENANT_WAREHOUSE_S3_*` config
/// (see [`Config::tenant_warehouse_s3_endpoint`]'s doc comment for why
/// these are dedicated settings, never `Config::rustfs_s3_endpoint`/
/// `Config::rustfs_access_key_secret_ref`).
///
/// Checked once, right before `provision_tenant`'s "pending" step would
/// otherwise attempt the Lakekeeper call — never partway through building
/// the request, so a deployment missing these settings, or pointing them
/// at a bucket that can never work, gets one honest 503 instead of a
/// request `Lakekeeper` would reject with a confusing validation error.
///
/// # Errors
///
/// [`crate::error::tenant_warehouse_storage_not_configured`] if
/// [`Config::tenant_warehouse_s3_endpoint`],
/// [`Config::tenant_warehouse_s3_access_key`], or
/// [`Config::tenant_warehouse_s3_secret_key`] is unset.
/// [`crate::error::tenant_warehouse_bucket_overlaps_default`] if
/// [`Config::tenant_warehouse_s3_bucket`] equals
/// [`Config::lakehouse_warehouse_bucket`] — every deployment's
/// `lakekeeper-warehouse-init` (`docker-compose.yml`) creates the shared
/// `default` warehouse at THAT bucket's storage-profile ROOT (no
/// `key-prefix`), so a tenant warehouse in the same bucket always 400s
/// with Lakekeeper's `CreateWarehouseStorageProfileOverlap`, regardless of
/// `key-prefix`. The remaining fields (region, path-style, STS) always
/// have a value — see their own doc comments — so they never gate this
/// check.
fn tenant_warehouse_storage(config: &Config) -> ApiResult<TenantWarehouseStorage> {
    let (Some(endpoint), Some(access_key), Some(secret_key)) = (
        config.tenant_warehouse_s3_endpoint.as_ref(),
        config.tenant_warehouse_s3_access_key.as_ref(),
        config.tenant_warehouse_s3_secret_key.as_ref(),
    ) else {
        return Err(crate::error::tenant_warehouse_storage_not_configured().into());
    };
    if config.tenant_warehouse_s3_bucket == config.lakehouse_warehouse_bucket {
        return Err(crate::error::tenant_warehouse_bucket_overlaps_default().into());
    }
    Ok(TenantWarehouseStorage {
        endpoint: endpoint.clone(),
        region: config.tenant_warehouse_s3_region.clone(),
        bucket: config.tenant_warehouse_s3_bucket.clone(),
        path_style_access: config.tenant_warehouse_s3_path_style_access,
        sts_enabled: config.tenant_warehouse_sts_enabled,
        sts_role_arn: config.tenant_warehouse_sts_role_arn.clone(),
        access_key: access_key.clone(),
        secret_key: Secret::new(secret_key.clone()),
    })
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


/// `PATCH /api/identity/users/{id}` — update a user's username and/or email (ADM-11 D8).
///
/// # Errors
///
/// 400 on malformed body, invalid username, or invalid email; 404 if user not found
/// or if caller does not have permission to rename an unrestricted user; 409 on clash.
pub async fn update_user(
    State(state): State<AppState>,
    AuthenticatedPrincipal(principal): AuthenticatedPrincipal,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<ApiJson<User>> {
    let user_id = Uuid::parse_str(&id)
        .map_err(|_| ApiError::BadRequest("user id must be a UUID".to_owned()))?;
    let pool = pool(&state)?;

    if !identity::user_exists(pool, user_id).await? {
        return Err(ApiError::NotFound("record not found".to_owned()).into());
    }

    let target_holds_unrestricted =
        lakehouse_auth::repository::user_holds_unrestricted(pool, user_id).await?;
    if target_holds_unrestricted && !crate::routes::catalog::is_unrestricted(&principal) {
        return Err(ApiError::NotFound("record not found".to_owned()).into());
    }

    let fields = &["username", "email"];
    let shape = "body must be an object with optional 'username' and 'email' fields";
    // ADM-11 review 1 S3: reuse routes::settings::parse_body rather than duplicating it.
    let sent = crate::routes::settings::parse_body(&body, fields, shape)?;

    let mut changed_fields = Vec::new();
    let new_username = if let Some(val) = sent.get("username") {
        if val.is_null() {
            return Err(ApiError::BadRequest("username cannot be null".to_owned()).into());
        }
        let u = val
            .as_str()
            .ok_or_else(|| ApiError::BadRequest("username must be a string".to_owned()))?;
        changed_fields.push("username".to_owned());
        Some(u)
    } else {
        None
    };

    let new_email = if let Some(val) = sent.get("email") {
        changed_fields.push("email".to_owned());
        if val.is_null() {
            lakehouse_store::identity::EmailUpdate::Clear
        } else {
            let e = val
                .as_str()
                .ok_or_else(|| ApiError::BadRequest("email must be a string or null".to_owned()))?;
            lakehouse_store::identity::EmailUpdate::Set(e)
        }
    } else {
        lakehouse_store::identity::EmailUpdate::Unchanged
    };

    let user = identity::update_user_identity(pool, user_id, new_username, new_email).await?;

    let audit_event = user_update_audit_event(&principal, user_id, &changed_fields);
    if let Err(err) = store_audit::insert(pool, audit_event).await {
        tracing::warn!(
            %err,
            action = "identity.user.update",
            user_id = %user_id,
            "failed to record user-update audit event"
        );
    }

    Ok(ApiJson(user))
}

fn user_update_audit_event(
    principal: &Principal,
    user_id: Uuid,
    changed_fields: &[String],
) -> NewAuditEvent {
    NewAuditEvent {
        action: "identity.user.update".to_owned(),
        resource_kind: Some("user".to_owned()),
        resource_id: Some(user_id.to_string()),
        principal_id: Some(principal.id.uuid().to_string()),
        principal_kind: Some(principal.kind_for_audit().to_owned()),
        actor_label: Some(principal.display_name.clone()),
        outcome: "executed".to_owned(),
        args: Some(serde_json::json!({
            "changedFields": changed_fields,
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
            // ADM-3 D2: the reset route is POST-only for the same reason
            // the rotate route is — a GET would hit axum's method
            // mismatch and never reach the auth gate.
            (
                "/api/identity/users/c4f7a7b2-0000-4000-8000-000000000001/reset-password",
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

    /// With every `TENANT_WAREHOUSE_S3_*` setting unset (`Config`'s
    /// default), `tenant_warehouse_storage` must refuse rather than build a
    /// request from empty strings.
    #[test]
    fn tenant_warehouse_storage_refuses_when_unconfigured() {
        let config = Config::from_map(&HashMap::new()).unwrap();
        let err = tenant_warehouse_storage(&config)
            .expect_err("unset TENANT_WAREHOUSE_S3_* must refuse, not fabricate a request");
        assert_eq!(err.0.status(), 503);
        assert!(
            err.0.to_string().contains("TENANT_WAREHOUSE_S3_ENDPOINT"),
            "the refusal should name the missing settings, got: {}",
            err.0
        );
    }

    /// With every setting present, and `TENANT_WAREHOUSE_S3_BUCKET`
    /// distinct from `LAKEHOUSE_WAREHOUSE_BUCKET`, `tenant_warehouse_storage`
    /// builds a [`TenantWarehouseStorage`] carrying exactly those values —
    /// the `bucket` field is the DEDICATED `TENANT_WAREHOUSE_S3_BUCKET`,
    /// never `LAKEHOUSE_WAREHOUSE_BUCKET` (see this function's doc comment
    /// for why the two must never be equal).
    #[test]
    fn tenant_warehouse_storage_builds_from_config_when_set() {
        let mut env = HashMap::new();
        env.insert(
            "TENANT_WAREHOUSE_S3_ENDPOINT".to_owned(),
            "http://rustfs.internal:9000".to_owned(),
        );
        env.insert(
            "TENANT_WAREHOUSE_S3_BUCKET".to_owned(),
            "acme-tenant-warehouses".to_owned(),
        );
        env.insert(
            "TENANT_WAREHOUSE_S3_ACCESS_KEY".to_owned(),
            "tenant-access-key".to_owned(),
        );
        env.insert(
            "TENANT_WAREHOUSE_S3_SECRET_KEY".to_owned(),
            "tenant-secret-key".to_owned(),
        );
        env.insert(
            "LAKEHOUSE_WAREHOUSE_BUCKET".to_owned(),
            "acme-warehouse".to_owned(),
        );
        let config = Config::from_map(&env).unwrap();
        let storage = tenant_warehouse_storage(&config).expect("every setting is present");
        assert_eq!(storage.endpoint, "http://rustfs.internal:9000");
        assert_eq!(storage.bucket, "acme-tenant-warehouses");
        assert_eq!(storage.access_key, "tenant-access-key");
        assert_eq!(storage.secret_key.expose(), "tenant-secret-key");
    }

    /// `TENANT_WAREHOUSE_S3_BUCKET` equal to `LAKEHOUSE_WAREHOUSE_BUCKET`
    /// (the bug the G2 gate's re-run exposed: this function used to read
    /// `lakehouse_warehouse_bucket` directly, always colliding with the
    /// `default` warehouse `lakekeeper-warehouse-init` creates at that
    /// bucket's root) must refuse, not build a request Lakekeeper would
    /// 400 on with `CreateWarehouseStorageProfileOverlap`.
    #[test]
    fn tenant_warehouse_storage_refuses_when_bucket_overlaps_the_default_warehouse() {
        let mut env = HashMap::new();
        env.insert(
            "TENANT_WAREHOUSE_S3_ENDPOINT".to_owned(),
            "http://rustfs.internal:9000".to_owned(),
        );
        env.insert(
            "TENANT_WAREHOUSE_S3_ACCESS_KEY".to_owned(),
            "tenant-access-key".to_owned(),
        );
        env.insert(
            "TENANT_WAREHOUSE_S3_SECRET_KEY".to_owned(),
            "tenant-secret-key".to_owned(),
        );
        // Same bucket on both settings — this is the collision, whatever
        // the actual name is.
        env.insert(
            "TENANT_WAREHOUSE_S3_BUCKET".to_owned(),
            "shared-warehouse".to_owned(),
        );
        env.insert(
            "LAKEHOUSE_WAREHOUSE_BUCKET".to_owned(),
            "shared-warehouse".to_owned(),
        );
        let config = Config::from_map(&env).unwrap();
        let err = tenant_warehouse_storage(&config)
            .expect_err("an overlapping bucket must refuse, not build a request");
        assert_eq!(err.0.status(), 503);
        assert!(
            err.0.to_string().contains("TENANT_WAREHOUSE_S3_BUCKET"),
            "the refusal should name the colliding setting, got: {}",
            err.0
        );
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
    /// Asserting the literal here is what pins down the "credential
    /// rotation is audited" property at the field level — the `INSERT`
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

    #[test]
    fn tenant_key_prefix_is_the_slug_alone_with_a_trailing_slash() {
        assert_eq!(tenant_key_prefix("acme"), "acme/");
        assert!(!tenant_key_prefix("acme-eu").starts_with(&tenant_key_prefix("acme")));
    }
}
