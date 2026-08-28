//! `/api/auth/*` — Task 3.2: login, logout, "who am I", and password
//! change. Wires `lakehouse-auth`'s primitives (never reimplements them)
//! onto this axum router.
//!
//! # Not a port
//!
//! No TypeScript backend route existed for any of this — the previous
//! backend authenticated nobody at all. Response shapes below are chosen
//! to be useful to a frontend, not bug-compatible with anything.
//!
//! # Non-enumeration on login
//!
//! [`login`] responds with the exact same status/shape whether the email
//! doesn't exist or the password is wrong — that guarantee lives in
//! [`lakehouse_auth::password::verify`] itself (see its module doc
//! comment), not duplicated here. What this module adds on top is a
//! single `tracing::warn!("login failed")` line with NO email and NO
//! secret in it, so an operator can see failed-login volume (and alert on
//! a spike) without this becoming a user-enumeration or credential-leak
//! oracle in the logs. A fuller rate limiter (e.g. per-IP/per-account
//! backoff) is future work, deliberately not built here: the load-bearing
//! anti-brute-force mitigation that already exists is
//! [`lakehouse_auth::password::verify`] paying the same `Argon2id` cost on
//! every attempt regardless of outcome, which already makes high-volume
//! guessing expensive; a counter on top would add complexity this task's
//! scope does not call for.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use lakehouse_auth::{Authenticator, Credential, PrincipalId, Secret, password, session};
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::identity;
use serde::{Deserialize, Serialize};
use time::Duration;

use crate::auth::{AuthenticatedPrincipal, SESSION_COOKIE_NAME, session_cookie_from_headers};
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::state::AppState;

/// Borrow the Postgres pool, or fail with the same 503 idiom
/// `routes::identity::pool` uses.
fn pool(state: &AppState) -> Result<&PgPool, ApiError> {
    state.pg.as_deref().ok_or_else(|| {
        ApiError::Unavailable(
            "authentication unavailable: no Postgres pool is configured (set DATABASE_URL)"
                .to_owned(),
        )
    })
}

/// Build the `Set-Cookie` header value carrying (or clearing) the session
/// token.
///
/// `Secure` is included unless [`crate::config::Config::is_dev`] — see
/// that field's doc comment for the exact env signal and why the default
/// fails closed to `Secure`. `HttpOnly` and `SameSite=Lax` are
/// unconditional: a `Lax` cookie is not sent on cross-site subrequests
/// (blocking a CSRF-via-`<img>`/fetch vector) while still being sent on a
/// top-level navigation, which is what a normal same-site SPA needs.
fn session_cookie_header(is_dev: bool, token: &str, ttl: Duration) -> HeaderValue {
    let max_age = ttl.whole_seconds().max(0);
    let secure = if is_dev { "" } else { "; Secure" };
    let raw = format!(
        "{SESSION_COOKIE_NAME}={token}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Lax{secure}"
    );
    // A hex token and this module's own literal characters can never fail
    // `HeaderValue` construction; the fallback exists only because
    // `unwrap`/`expect` are denied outside tests, not because this is
    // expected to trigger.
    HeaderValue::from_str(&raw).unwrap_or_else(|_| {
        HeaderValue::from_static("lh_session=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax")
    })
}

/// The `Set-Cookie` value that clears the session cookie (`logout`).
fn cleared_session_cookie_header(is_dev: bool) -> HeaderValue {
    session_cookie_header(is_dev, "", Duration::ZERO)
}

/// `{ email, password }` — `POST /api/auth/login` body.
#[derive(Debug, Deserialize)]
struct LoginBody {
    email: String,
    password: String,
}

/// `POST /api/auth/login` response body.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LoginResponse {
    id: String,
    name: String,
    must_change_password: bool,
}

/// `POST /api/auth/login` — verify `{ email, password }`, and on success,
/// create a server-side session and set its cookie.
///
/// # Errors
///
/// Returns the caller's [`ApiError`] (400 on an unparseable body, 401 on
/// bad credentials — see the module doc comment for why that one status is
/// shared between "no such email" and "wrong password") via `?`.
pub async fn login(State(state): State<AppState>, body: Bytes) -> ApiResult<Response> {
    let pool = pool(&state)?;
    let auth = state.auth.as_ref().ok_or_else(|| {
        ApiError::Unavailable(
            "authentication unavailable: no Postgres pool is configured (set DATABASE_URL)"
                .to_owned(),
        )
    })?;
    let LoginBody { email, password } = serde_json::from_slice(&body)
        .map_err(|err| ApiError::BadRequest(format!("invalid JSON body: {err}")))?;

    // Goes through `LocalPasswordAuthenticator` (not
    // `lakehouse_auth::password::verify` directly) so this route exercises
    // exactly the same `Authenticator` seam every other credential kind
    // does — see `crate::auth`'s module doc comment.
    let credential = Credential::Password {
        identifier: email,
        password: Secret::new(password),
    };
    let principal = auth
        .local
        .authenticate(&credential)
        .await
        .inspect_err(|_| tracing::warn!("login failed"))?;

    let PrincipalId::User(user_id) = principal.id else {
        // `password::verify` only ever resolves a `PrincipalId::User` (it
        // loads through `app_user`) — this branch exists so the match is
        // exhaustive, not because it can be reached in practice.
        tracing::warn!("login failed");
        return Err(ApiError::invalid_or_expired().into());
    };

    let must_change_password = password::must_change_password(pool, user_id)
        .await
        .unwrap_or(false);
    let token =
        session::create_session(pool, user_id, session::DEFAULT_SESSION_TTL, None, None).await?;

    let body = LoginResponse {
        id: user_id.to_string(),
        name: principal.display_name,
        must_change_password,
    };
    let mut response = (StatusCode::OK, ApiJson(body)).into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        session_cookie_header(
            state.config.is_dev,
            token.expose(),
            session::DEFAULT_SESSION_TTL,
        ),
    );
    Ok(response)
}

/// `POST /api/auth/logout` — revoke the caller's session server-side (so a
/// replayed copy of the old cookie value is rejected, not just the
/// browser's copy cleared) and clear the cookie.
///
/// # Errors
///
/// Returns a 503 if no Postgres pool is configured, or the underlying
/// [`lakehouse_auth::AuthError`] on a storage failure.
pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
    AuthenticatedPrincipal(_principal): AuthenticatedPrincipal,
) -> ApiResult<Response> {
    let pool = pool(&state)?;
    if let Some(token) = session_cookie_from_headers(&headers) {
        session::revoke_session(pool, &Secret::new(token)).await?;
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        cleared_session_cookie_header(state.config.is_dev),
    );
    Ok(response)
}

/// `GET /api/auth/me` response body.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MeResponse {
    id: String,
    name: String,
    /// `None` for a service principal (no `app_user` row), or if the
    /// backing `app_user` row could not be re-read after authentication
    /// succeeded — see the doc comment on this handler for the gap this
    /// documents.
    email: Option<String>,
    /// Role *names*, from `lakehouse_store::identity` — empty for a
    /// service principal (service credentials have scopes, not roles).
    roles: Vec<String>,
    /// Every granted `"resource:action"` permission token (see
    /// [`lakehouse_auth::PermissionSet::as_strings`]).
    permissions: Vec<String>,
    tenants: Vec<String>,
}

/// `GET /api/auth/me` — the authenticated caller's own identity, shaped
/// for a frontend to render (e.g. gating UI on `permissions`).
///
/// # A documented gap
///
/// [`lakehouse_auth::Principal`] deliberately carries no `email` (see its
/// doc comment — it is a normalized shape every [`lakehouse_auth::Authenticator`]
/// produces identically, and a service principal has no email at all). For a
/// [`lakehouse_auth::PrincipalId::User`], this handler re-reads
/// `app_user`/its role names via `lakehouse_store::identity::get_user` to
/// fill `email`/`roles` in; that is a second query beyond what
/// authentication itself needed, and if it fails (a race with the account
/// being deleted between authenticating and this read, in practice) `email`
/// degrades to `None` and `roles` to empty rather than failing the whole
/// request — the caller is still who [`AuthenticatedPrincipal`] says they
/// are.
///
/// # Errors
///
/// Cannot fail on its own; [`AuthenticatedPrincipal`]'s extraction is what
/// produces the 401/503 for this route.
pub async fn me(
    State(state): State<AppState>,
    AuthenticatedPrincipal(principal): AuthenticatedPrincipal,
) -> ApiResult<Response> {
    let (email, roles) = match principal.id {
        PrincipalId::User(user_id) => match state.pg.as_deref() {
            Some(pool) => match identity::get_user(pool, &user_id.to_string()).await {
                Ok(user) => (Some(user.email), user.roles),
                Err(_) => (None, Vec::new()),
            },
            None => (None, Vec::new()),
        },
        PrincipalId::Service(_) => (None, Vec::new()),
    };

    let body = MeResponse {
        id: principal.id.uuid().to_string(),
        name: principal.display_name,
        email,
        roles,
        permissions: principal.permissions.as_strings(),
        tenants: principal
            .tenant_ids
            .iter()
            .map(ToString::to_string)
            .collect(),
    };
    Ok((StatusCode::OK, ApiJson(body)).into_response())
}

/// `{ oldPassword?, newPassword }` — `POST /api/auth/change-password` body.
/// `oldPassword` is optional ONLY because it is not read at all for a
/// forced (`must_change_password`) rotation — see [`change_password`].
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChangePasswordBody {
    #[serde(default)]
    old_password: Option<String>,
    new_password: String,
}

/// `POST /api/auth/change-password` — authenticated. Verifies the caller's
/// current password UNLESS their account is flagged
/// `must_change_password` (a bootstrapped credential, per
/// `0019_auth.sql`/`lakehouse_auth::password`'s doc comments), in which
/// case the forced rotation is allowed through without re-proving the
/// bootstrap password — the whole point of that flag is that the
/// bootstrap credential is treated as "good for logging in and nothing
/// else" until it's replaced. Revokes every other live session for the
/// account afterward (mirrors `lakehouse_auth::session::revoke_all_sessions_for_user`'s
/// own doc comment: a credential rotation should not leave old sessions
/// valid), including the one used to make this call.
///
/// # Errors
///
/// Returns 400 on an unparseable body or a missing `oldPassword` when one
/// is required; the caller's [`ApiError`] (401) if `oldPassword` does not
/// verify; 503 if no Postgres pool is configured; or the underlying
/// storage error on any other failure.
pub async fn change_password(
    State(state): State<AppState>,
    AuthenticatedPrincipal(principal): AuthenticatedPrincipal,
    body: Bytes,
) -> ApiResult<Response> {
    let pool = pool(&state)?;
    let PrincipalId::User(user_id) = principal.id else {
        return Err(ApiError::BadRequest(
            "only a human account can change its password".to_owned(),
        )
        .into());
    };

    let ChangePasswordBody {
        old_password,
        new_password,
    } = serde_json::from_slice(&body)
        .map_err(|err| ApiError::BadRequest(format!("invalid JSON body: {err}")))?;

    let must_rotate = password::must_change_password(pool, user_id)
        .await
        .unwrap_or(false);
    if !must_rotate {
        let Some(old_password) = old_password else {
            return Err(ApiError::BadRequest("oldPassword is required".to_owned()).into());
        };
        let user = identity::get_user(pool, &user_id.to_string()).await?;
        password::verify(pool, &user.email, &Secret::new(old_password))
            .await
            .inspect_err(|_| {
                tracing::warn!("change-password rejected: old password did not verify");
            })?;
    }

    password::change_password(pool, user_id, &Secret::new(new_password)).await?;
    session::revoke_all_sessions_for_user(pool, user_id).await?;

    Ok(StatusCode::NO_CONTENT.into_response())
}
