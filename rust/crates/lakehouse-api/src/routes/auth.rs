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
//! comment), not duplicated here. What this module adds on top:
//!
//! * A single `tracing::warn!("login failed")` line with NO email and NO
//!   secret in it, so an operator can see failed-login volume (and alert on
//!   a spike) without this becoming a user-enumeration or credential-leak
//!   oracle in the logs.
//! * Per-account throttle (SEC-2): `N` wrong passwords within a window lock
//!   the key for the lockout duration. A locked key returns `429` with a
//!   `Retry-After` header and never reaches password verification — the
//!   lockout response is identical whether the account exists or not.
//!   The defaults (5 failures / 900 s → 300 s lock) are env settings
//!   (`LOGIN_MAX_FAILURES`, `LOGIN_FAILURE_WINDOW_SECS`, `LOGIN_LOCKOUT_SECS`).
//!
//! See `docs/core/features/login-protection-and-session-cleanup.md` for
//! the full specification and acceptance checklist.

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use lakehouse_auth::throttle;
use lakehouse_auth::{
    AuthError, Authenticator, Credential, Principal, PrincipalId, Secret, password, session,
};
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::StoreError;
use lakehouse_store::audit::{self as store_audit, NewAuditEvent};
use lakehouse_store::identity;
use lakehouse_store::sessions::SessionRow;
use serde::{Deserialize, Serialize};
use sha2::Digest as _;
use time::Duration;
use uuid::Uuid;

use crate::auth::{AuthenticatedPrincipal, SESSION_COOKIE_NAME, session_cookie_from_headers};
use crate::error::{ApiRejection, ApiResult};
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

/// `{ identifier, password }` — `POST /api/auth/login` body.
/// Accepts `email` as an alias for backwards compatibility (ADM-11 D6).
#[derive(Debug, Deserialize)]
struct LoginBody {
    #[serde(alias = "email")]
    identifier: String,
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

/// The one answer for a locked key, shared by the lock check and the
/// over-the-limit refusal (SEC-49) so the two cannot differ on the wire.
/// `None` (no lock time known) waits one second.
fn locked_response(locked_until: Option<time::OffsetDateTime>) -> ApiError {
    // `.max(1)` keeps the value in range, so the `try_from` fallback is
    // unreachable; it exists so no `as` cast can silently wrap a negative
    // difference.
    let secs = locked_until.map_or(1, |locked| {
        let now = time::OffsetDateTime::now_utc();
        u64::try_from((locked - now).whole_seconds() + 1)
            .unwrap_or(1)
            .max(1)
    });
    ApiError::TooManyRequests {
        message: "Too many failed sign-in attempts. Try again later.".to_owned(),
        retry_after_secs: secs,
    }
}

/// The longest `User-Agent` a session records, in characters. A browser's
/// header is short; the cap keeps a caller from storing a kilobyte of text
/// under the column the Sessions page renders.
const SESSION_USER_AGENT_MAX_CHARS: usize = 512;

/// The `User-Agent` a sign-in records on its session, or `None`
/// (`QA LH-SEC-5-01`, `ADM-14` decision D9).
///
/// It is what the client says about itself: shown on the Sessions page,
/// never trusted for anything. Trimmed, cut to
/// [`SESSION_USER_AGENT_MAX_CHARS`], and nothing when the header is absent,
/// empty, or carries bytes that are not plain ASCII (`to_str` refuses
/// those, and a lossy conversion would invent text).
fn session_user_agent(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::USER_AGENT)?.to_str().ok()?.trim();
    if raw.is_empty() {
        return None;
    }
    Some(raw.chars().take(SESSION_USER_AGENT_MAX_CHARS).collect())
}

/// `POST /api/auth/login` — verify `{ email, password }`, and on success,
/// create a server-side session and set its cookie.
///
/// # Throttle (SEC-2)
///
/// Before any password verification, the email is hashed and checked
/// against `login_throttle`. If the key is locked, the route returns `429`
/// with a `Retry-After` header and the same fixed message for an existing
/// and a non-existing account — the hash is computed identically regardless
/// of whether an account with that email exists, and a locked key never
/// reaches password verification.
///
/// Each try is counted by `record_attempt` before the password is checked
/// (SEC-49), so concurrent tries cannot exceed the limit; a try over the
/// limit is refused without a hash. If a try reaches
/// the limit and fails, one `auth.login_locked` audit row is written
/// (best-effort: a failed audit write is logged, not propagated). On a successful login,
/// `clear` resets the count.
///
/// # Errors
///
/// 400 on an unparseable body; 401 on bad credentials (same status for
/// "no such email" and "wrong password" — see the module doc comment);
/// 429 when the key is locked; 503 if no Postgres pool is configured.
pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Response> {
    let pool = pool(&state)?;
    let auth = state.auth.as_ref().ok_or_else(|| {
        ApiError::Unavailable(
            "authentication unavailable: no Postgres pool is configured (set DATABASE_URL)"
                .to_owned(),
        )
    })?;
    let LoginBody {
        identifier,
        password,
    } = serde_json::from_slice(&body)
        .map_err(|err| ApiError::BadRequest(format!("invalid JSON body: {err}")))?;

    let key = throttle::key_for(&identifier);

    if let Some(locked_until) = throttle::locked_until(pool, &key).await? {
        return Err(locked_response(Some(locked_until)).into());
    }

    // SEC-49: count this try BEFORE the password is hashed. Counting only
    // after a failed check let many simultaneous guesses pass the lock check
    // above before the first failure was written. The UPSERT serialises in
    // Postgres, so only the first `max_failures` concurrent tries get a
    // count within the limit and reach Argon2; the rest answer exactly as
    // a locked key does, without a hash. A missing user still hashes
    // (`DUMMY_HASH`) while within the limit, so timing is unchanged.
    //
    // `ADM-8`: the policy is resolved once per try, so every number this
    // try uses — the count, the limit and the refund — comes from the same
    // answer, even if a save lands in the console while the try runs.
    let policy = state.throttle_policy().await;
    let (attempt, attempt_lock) = throttle::record_attempt(pool, &key, &policy).await?;
    if attempt > policy.max_failures {
        return Err(locked_response(attempt_lock).into());
    }

    let credential = Credential::Password {
        identifier,
        password: Secret::new(password),
    };
    match auth.local.authenticate(&credential).await {
        Ok(principal) => {
            throttle::clear(pool, &key).await?;
            let PrincipalId::User(user_id) = principal.id else {
                tracing::warn!("login failed");
                return Err(ApiError::Unauthorized(
                    "Invalid username/email or password.".to_owned(),
                )
                .into());
            };
            let must_change_password = password::must_change_password(pool, user_id)
                .await
                .unwrap_or(false);
            let token = session::create_session(
                pool,
                user_id,
                session::DEFAULT_SESSION_TTL,
                // ADM-14 D10: no address — the API sits behind the console's
                // own server and has no trusted source for the client's.
                None,
                session_user_agent(&headers).as_deref(),
                &[],
            )
            .await?;
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
        // Only a rejected password counts toward the lock, and only it
        // renders as the non-enumerated 401: a storage error mid-login
        // must reach the caller as its classified error (see
        // `From<AuthError> for ApiError` — a fixed "authentication error"
        // 500), never as "wrong password", and must never walk a real
        // user toward a lockout over a database hiccup (review blocker 1,
        // 2026-10-03).
        Err(AuthError::InvalidCredentials) => {
            // SEC-49: the try was already counted before hashing; the try
            // that reached the limit is the one that set the lock.
            if attempt_lock.is_some() && attempt == policy.max_failures {
                let event = NewAuditEvent {
                    action: "auth.login_locked".to_owned(),
                    outcome: "refused".to_owned(),
                    resource_kind: Some("login_key".to_owned()),
                    resource_id: Some(key[..12].to_owned()),
                    // QA LH-SEC-2-06 (SEC-70, decision D3): nobody signed in
                    // yet, so the event has no principal and names the form
                    // that produced it.
                    actor_label: Some(lakehouse_store::audit::ACTOR_SIGN_IN_FORM.to_owned()),
                    ..Default::default()
                };
                if let Err(err) = store_audit::insert(pool, event).await {
                    tracing::warn!(%err, "failed to record auth.login_locked audit event");
                }
            }
            tracing::warn!("login failed");
            Err(ApiError::Unauthorized("Invalid username/email or password.".to_owned()).into())
        }
        Err(err) => {
            // SEC-49: the try was counted before hashing; a storage error is
            // not a wrong password, so give the count back (best-effort).
            // The refund also takes away a lock this very try set (PR #236
            // review SHOULD-FIX 1): the try that reaches the limit locks
            // the key, and a storage error after that must not leave a real
            // user locked out over a database hiccup.
            if let Err(refund_err) = throttle::refund_attempt(pool, &key, &policy).await {
                tracing::warn!(%refund_err, "failed to refund a login attempt");
            }
            tracing::warn!(error = %err, "login failed with a non-credential error");
            Err(ApiError::from(err).into())
        }
    }
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
    /// Structured `{id, name, slug}` view of every tenant the caller
    /// belongs to — slim subset of `lakehouse_store::identity::Tenant`,
    /// just enough for a tenant-switcher to render a label without
    /// re-fetching the row. Empty for a service principal
    /// (no `tenant_ids`) or when the follow-up read fails — same
    /// fail-soft posture as `email`/`roles` above.
    tenants: Vec<identity::TenantSummary>,
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
                // ADM-11 D6: a user may have no email at all, so the
                // lookup's success no longer implies an address exists —
                // `None` means the account has none.
                Ok(user) => (user.email, user.roles),
                Err(_) => (None, Vec::new()),
            },
            None => (None, Vec::new()),
        },
        PrincipalId::Service(_) => (None, Vec::new()),
    };

    // Follow-up read so a caller of `GET /api/auth/me` gets each tenant's
    // `name` and `slug` alongside its id — a tenant-switcher renders the
    // label, so a bare-UUID-string array would not be enough. Same
    // fail-soft posture as `email`/`roles` above (degrade to empty on
    // failure), and `list_tenants_by_ids` short-circuits internally when
    // `principal.tenant_ids` is empty — so a service principal, which has
    // no `tenant_ids` at all (`lakehouse_auth::service_token` initializes
    // it to `Vec::new()`), costs zero queries on this branch.
    let tenants = match state.pg.as_deref() {
        Some(pool) => identity::list_tenants_by_ids(pool, &principal.tenant_ids)
            .await
            .unwrap_or_default(),
        None => Vec::new(),
    };

    let body = MeResponse {
        id: principal.id.uuid().to_string(),
        name: principal.display_name,
        email,
        roles,
        permissions: principal.permissions.as_strings(),
        tenants,
    };
    Ok((StatusCode::OK, ApiJson(body)).into_response())
}

/// What `GET /api/auth/sessions` should return for `principal`. The
/// handler dispatches on this rather than threading two booleans through
/// the SQL, so the service-principal branch ("a service identity has no
/// `app_user_id` of its own to filter by") is a single match arm and a
/// single unit-test target — rather than a second conditional in the
/// query path that an integration test would only catch by accident.
///
/// Admin-vs-own is decided by `principal.has("identity:sessions:manage")`:
/// this route lists only the caller's own sessions unless the caller holds
/// that permission, in which case it lists every live session in the
/// deployment. The route-level policy is
/// `Policy::RequiresAuth` (every authenticated caller can hit it), and
/// the fine-grained check lives in the handler so the same permission
/// string covers any future endpoint that needs the same split without
/// re-adding a row to `POLICY_TABLE`.
#[derive(Debug, PartialEq, Eq)]
enum SessionsDecision {
    /// A service principal has no `app_user.id` of its own to enumerate —
    /// including when it holds `identity:sessions:manage`; the admin pass
    /// is "every `app_user.id`", and a service identity has nothing of
    /// that kind. Returning `Empty` here is the same "service principals
    /// never enumerate" posture every other handler in this file keeps.
    Empty,
    /// Caller's own sessions only — the default.
    Own(Uuid),
    /// Every live session — the `identity:sessions:manage` permission
    /// flips the `is_admin` argument on
    /// [`lakehouse_store::sessions::list_sessions_for_caller`].
    All(Uuid),
}

fn sessions_decision_for(principal: &Principal) -> SessionsDecision {
    match principal.id {
        PrincipalId::Service(_) => SessionsDecision::Empty,
        PrincipalId::User(user_id) if principal.has("identity:sessions:manage") => {
            SessionsDecision::All(user_id)
        }
        PrincipalId::User(user_id) => SessionsDecision::Own(user_id),
    }
}

/// `GET /api/auth/sessions` — list the caller's own live browser sessions,
/// or every live session in the deployment if the caller holds
/// `identity:sessions:manage`. Route-level policy is `Policy::RequiresAuth`
/// (every authenticated caller may hit it — the admin-vs-own split lives
/// here in the handler, see [`SessionsDecision`]).
///
/// # What each row records
///
/// `user_agent` is the browser as it named itself at sign-in (`ADM-14`
/// decision D9), and `null` for a session older than that change.
/// `created_ip` is always `null`: the API sits behind the console's own
/// server and has no trusted source for the client's address, so the page
/// says "Not recorded" rather than showing a wrong address as a fact
/// (decision D10) — see `lakehouse_store::sessions::SessionRow`'s own doc
/// comment.
///
/// # Errors
///
/// Returns 503 [`ApiError::Unavailable`] when no Postgres pool is
/// configured (mirroring [`pool`]'s idiom), or a classified
/// [`lakehouse_store::StoreError`] on any storage failure.
pub async fn sessions(
    State(state): State<AppState>,
    AuthenticatedPrincipal(principal): AuthenticatedPrincipal,
) -> ApiResult<ApiJson<Vec<SessionRow>>> {
    let pool = pool(&state)?;
    let rows = match sessions_decision_for(&principal) {
        SessionsDecision::Empty => Vec::new(),
        SessionsDecision::Own(user_id) => {
            lakehouse_store::sessions::list_sessions_for_caller(pool, user_id, false).await?
        }
        SessionsDecision::All(user_id) => {
            lakehouse_store::sessions::list_sessions_for_caller(pool, user_id, true).await?
        }
    };
    Ok(ApiJson(rows))
}

/// `DELETE /api/auth/sessions/{id}` — revoke one live browser session. An
/// `identity:sessions:manage` holder may revoke any session; everyone else
/// may revoke only their own.
///
/// The route never tells a caller which of "your session",
/// "another user's session", and "no such session" they hit — it never
/// reveals whether another user's session id exists. All three collapse
/// to the same 404 with the same body, so
/// the route is not an enumeration oracle:
///
/// * A foreign id (someone else's session, caller is not admin) and a
///   missing id both arrive at [`lakehouse_store::sessions::revoke_session_as_caller`]
///   as `SessionOwnershipError::NotFound` (the SQL's
///   `WHERE id = $1 AND revoked_at IS NULL AND ($3 OR app_user_id = $2)`
///   predicate matches zero rows either way).
/// * A service principal has no `app_user.id` of its own to enumerate —
///   same posture as `SessionsDecision::Empty` on the list endpoint — so
///   it short-circuits to the same 404 body before touching the DB. The
///   service-principal branch carries the same `ApiError::NotFound`
///   message as `StoreError::NotFound` would render, so the bytes match
///   (verified by [`tests::delete_session_route_404_body_for_foreign_id_matches_missing_id`]).
///
/// A non-UUID path segment is a 400, not a 404: a malformed id can't
/// match any row, so 404'ing it would be a "doesn't exist" response that
/// also can't be a "foreign-id" check — exactly the wrong shape for a
/// caller that's misusing the API. The 400 makes the malformed-path
/// case its own shape.
///
/// `Policy::RequiresAuth` is the floor (every authenticated caller may
/// hit this), and the admin-vs-own split lives here in the handler, the
/// same way [`sessions`] splits it for the list endpoint.
///
/// # Errors
///
/// Returns 400 [`ApiError::BadRequest`] on a non-UUID `{id}`; 404
/// [`ApiError::NotFound`] uniformly for foreign / missing / already-
/// revoked ids and for service principals; 503
/// [`ApiError::Unavailable`] when no Postgres pool is configured; or a
/// classified [`StoreError`] (via [`SessionOwnershipError`]'s own
/// `From<...> for StoreError` mapping) on any storage failure.
pub async fn revoke_session(
    State(state): State<AppState>,
    AuthenticatedPrincipal(principal): AuthenticatedPrincipal,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let session_id = Uuid::parse_str(&id)
        .map_err(|_| ApiError::BadRequest("session id must be a UUID".to_owned()))?;

    let PrincipalId::User(caller_id) = principal.id else {
        return Err(ApiError::NotFound("record not found".to_owned()).into());
    };
    let is_admin = principal.has("identity:sessions:manage");

    let pool = pool(&state)?;
    lakehouse_store::sessions::revoke_session_as_caller(pool, session_id, caller_id, is_admin)
        .await
        .map_err(StoreError::from)?;

    // audit_event row mirrors 0024_audit_event.sql's schema — `outcome = "executed"`
    // per `audit_event_outcome_check` (0024_audit_event.sql). `resource_id`
    // is the request path's `{id}` (the session row's UUID, never the
    // caller's bearer token — the token is not yet validated at this
    // point and the route never sees it). Best-effort: a failed audit
    // write is logged and swallowed, never propagated, so the response
    // code on the wire is always the underlying revoke's, not the
    // audit's.
    let audit_event = session_revoke_audit_event(&principal, session_id);
    if let Err(err) = store_audit::insert(pool, audit_event).await {
        tracing::warn!(
            %err,
            action = "auth.session.revoke",
            session_id = %session_id,
            "failed to record session revoke audit event"
        );
    }

    Ok(StatusCode::NO_CONTENT.into_response())
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
        password::verify_for_user(pool, user_id, &Secret::new(old_password))
            .await
            .inspect_err(|_| {
                tracing::warn!("change-password rejected: old password did not verify");
            })?;
    }

    let user = identity::get_user(pool, &user_id.to_string()).await?;
    password::check_rules(&new_password, Some(&user.username), user.email.as_deref())
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;

    password::change_password(pool, user_id, &Secret::new(new_password)).await?;
    session::revoke_all_sessions_for_user(pool, user_id).await?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Validate `path` as a same-origin, in-app relative path — the one
/// server-side defense against an open redirect via `?next=`. Mirrors
/// `src/features/auth/login-page.tsx`'s `nextPathFromQuery` client-side
/// check but is the check that actually matters: this server issues the
/// redirect, the client-side one is only UX.
///
/// # Why a leading-`/` check alone is not enough
///
/// `path.starts_with('/') && !path.starts_with("//")` alone still admits
/// `/\evil.invalid` — several browsers normalize a backslash toward a
/// forward slash before resolving a URL, which turns `/\evil.invalid`
/// into the scheme-relative `//evil.invalid` this function is already
/// trying to block — the backslash is not a same-origin path character in
/// this context, it is an alternate spelling of the attack this function
/// exists to stop. Rejecting `\` anywhere in the path (not only a leading
/// `/\`) closes both the leading form and an embedded one
/// (`/legit/path\evil.invalid`).
fn is_safe_relative_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.starts_with("//")
        && !path.contains('\\')
        && !path.starts_with("/login")
}

/// Fold an untrusted `?next=` query value down to a value this service
/// will actually redirect to after login: the value itself if
/// [`is_safe_relative_path`] accepts it, otherwise the default `"/"` — a
/// rejected value is discarded, never re-encoded into the flow cookie (see
/// [`OidcFlow`]'s doc comment for why that distinction matters).
fn safe_return_path(raw: Option<&str>) -> String {
    match raw {
        Some(path) if is_safe_relative_path(path) => path.to_owned(),
        _ => "/".to_owned(),
    }
}

/// The `lh_oidc_flow` cookie's contents: everything `/callback` needs to
/// validate the response and finish the login, bundled into one
/// `HttpOnly`/`Secure`/`SameSite=Lax` cookie rather than four separate
/// short-lived server-side rows — this value is single-use and lives for
/// at most [`OIDC_FLOW_TTL_SECONDS`], so a Postgres row (with its own
/// cleanup job) would be more machinery than the risk being managed calls
/// for.
///
/// `next` is stored here, NOT re-derived from the request at `/callback`
/// time, precisely so a rejected `?next=` (see [`safe_return_path`]) can
/// never reach the cookie in the first place — the open-redirect surface
/// stays confined to this one function's validation, not duplicated at
/// two call sites that could drift.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct OidcFlow {
    pub(crate) state: String,
    pub(crate) nonce: String,
    pub(crate) code_verifier: String,
    pub(crate) next: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) test_run_id: Option<Uuid>,
}

const OIDC_FLOW_COOKIE_NAME: &str = "lh_oidc_flow";
const OIDC_FLOW_TTL_SECONDS: i64 = 600;

/// Build the `Set-Cookie` header carrying (`Some`) or clearing (`None`)
/// the `lh_oidc_flow` cookie. `Secure` follows the exact same condition
/// [`session_cookie_header`] uses — see that function's doc comment for
/// why the default fails closed to `Secure`.
pub(crate) fn oidc_flow_cookie_header(is_dev: bool, flow: Option<&OidcFlow>) -> HeaderValue {
    let (value, max_age) = match flow {
        Some(flow) => {
            let json = serde_json::to_string(flow).unwrap_or_default();
            (
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json),
                OIDC_FLOW_TTL_SECONDS,
            )
        }
        None => (String::new(), 0),
    };
    let secure = if is_dev { "" } else { "; Secure" };
    let raw = format!(
        "{OIDC_FLOW_COOKIE_NAME}={value}; Path=/api/auth/oidc; Max-Age={max_age}; HttpOnly; SameSite=Lax{secure}"
    );
    // Base64url-encoded JSON plus this function's own literal characters
    // can never fail `HeaderValue` construction; the fallback exists only
    // because `unwrap`/`expect` are denied outside tests, not because
    // this is expected to trigger.
    HeaderValue::from_str(&raw).unwrap_or_else(|_| {
        HeaderValue::from_static(
            "lh_oidc_flow=; Path=/api/auth/oidc; Max-Age=0; HttpOnly; SameSite=Lax",
        )
    })
}

/// `GET /api/auth/oidc/start` query parameters — just the one optional,
/// untrusted `next` value; see [`safe_return_path`] for how it's turned
/// into something safe to store.
#[derive(Debug, Deserialize)]
pub struct OidcStartQuery {
    #[serde(default)]
    next: Option<String>,
}

/// `GET /api/auth/oidc/start?next=` — begin an authorization-code + PKCE
/// login. Unauthenticated by design (`Policy::Public` — see
/// `crate::policy::POLICY_TABLE`'s entry for this route: it only ever
/// *redirects* to a fixed, config-sourced `IdP` URL and sets a single-use
/// cookie; it grants nothing).
///
/// # Errors
///
/// Returns 503 if OIDC is not configured (`AuthState::oidc` is `None`, or
/// `Config::oidc_authorize_url`/`oidc_redirect_uri` is unset) — the same
/// "unconfigured, not broken" signal `routes::identity::pool` gives for a
/// missing Postgres pool.
pub async fn oidc_start(
    State(state): State<AppState>,
    Query(query): Query<OidcStartQuery>,
) -> ApiResult<Response> {
    // `ADM-12` D10: the runtime, not the bare config. It carries the
    // addresses the provider's discovery document named, and is ready only
    // when the connection can sign someone in — so a saved connection with
    // no return address or an unreachable discovery answers a plain 503
    // here instead of redirecting somewhere that cannot work. Review round
    // 2, R2-B1: an authenticator without sign-in addresses (a bearer-only
    // deployment) is not enough for this route.
    let runtime = crate::runtime_settings::sso_runtime(&state).await;
    let Some(_auth) = runtime.sign_in_authenticator() else {
        return Err(ApiError::Unavailable(
            runtime
                .not_ready_reason()
                .unwrap_or("Single sign-on is not configured on this deployment.")
                .to_owned(),
        )
        .into());
    };
    let sso = runtime.settings;
    let authorize_url = sso
        .authorize_url
        .as_deref()
        .ok_or_else(|| ApiError::Unavailable("OIDC_AUTHORIZE_URL is not configured".to_owned()))?;
    let redirect_uri = sso
        .redirect_uri
        .as_deref()
        .ok_or_else(|| ApiError::Unavailable("OIDC_REDIRECT_URI is not configured".to_owned()))?;
    let client_id = sso
        .client_id
        .as_deref()
        .ok_or_else(|| ApiError::Unavailable("OIDC_CLIENT_ID is not configured".to_owned()))?;

    // `state`/`nonce`/PKCE `code_verifier` all come from the same CSPRNG
    // helper every other bearer-equivalent secret in this codebase uses
    // (`lakehouse_auth::generate_opaque_token`) — never an ad-hoc `rand`
    // call here. 64 hex characters is within RFC 7636's 43-128 character
    // range for a PKCE code verifier.
    let state_token = lakehouse_auth::generate_opaque_token();
    let nonce = lakehouse_auth::generate_opaque_token();
    let code_verifier = lakehouse_auth::generate_opaque_token();
    let challenge_digest = sha2::Sha256::digest(code_verifier.expose().as_bytes());
    let code_challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(challenge_digest);

    let flow = OidcFlow {
        state: state_token.expose().to_owned(),
        nonce: nonce.expose().to_owned(),
        code_verifier: code_verifier.expose().to_owned(),
        next: safe_return_path(query.next.as_deref()),
        test_run_id: None,
    };

    let mut redirect = url::Url::parse(authorize_url)
        .map_err(|_| ApiError::Unavailable("OIDC_AUTHORIZE_URL is not a valid URL".to_owned()))?;
    redirect
        .query_pairs_mut()
        .append_pair("response_type", "code")
        // Not `unwrap_or_default()`: an empty `client_id` would still be
        // sent, and the flow would fail at the `IdP` with an error this
        // service never sees. `AuthState::oidc` is `Some` only when
        // `OIDC_ISSUER` *and* `OIDC_CLIENT_ID` are both set
        // (`state::oidc_config`, which `?`s on each), so the presence
        // check above already guarantees this value — reading it the same
        // fail-closed way as the other three keeps that guarantee true if
        // `oidc_config`'s requirements ever change.
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("scope", "openid email profile")
        .append_pair("state", &flow.state)
        .append_pair("nonce", &flow.nonce)
        .append_pair("code_challenge", &code_challenge)
        .append_pair("code_challenge_method", "S256");

    let mut response = (StatusCode::FOUND, ()).into_response();
    response.headers_mut().insert(
        header::LOCATION,
        HeaderValue::from_str(redirect.as_str()).map_err(|_| {
            ApiError::Unavailable("could not build the OIDC authorize redirect".to_owned())
        })?,
    );
    response.headers_mut().append(
        header::SET_COOKIE,
        oidc_flow_cookie_header(state.config.is_dev, Some(&flow)),
    );
    Ok(response)
}

/// `GET /api/auth/providers` response body: just enough for a login page
/// to decide whether to render an SSO button and what to label it, never
/// the configuration values themselves (no issuer URL, no client id).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvidersResponse {
    oidc: bool,
    provider_name: Option<String>,
}

/// `GET /api/auth/providers` — replaces the build-time
/// `NEXT_PUBLIC_SSO_ENABLED` flag with a runtime read of whether OIDC is
/// actually configured on THIS API process, so a login page's SSO button
/// can never drift from the backend's real state (a build-time flag could
/// say "enabled" on a deployment where `OIDC_ISSUER`/`OIDC_CLIENT_ID` were
/// never set, or vice versa). `Policy::Public` (see
/// `crate::policy::POLICY_TABLE`'s entry for this route): this reveals
/// only a boolean and a label an operator already chose to be
/// public-facing (`OIDC_PROVIDER_NAME`), never a secret or the issuer/
/// client id `AuthState::oidc`/`Config` also carry — and the login page
/// needs this before any session exists, same as `/api/auth/login` and
/// the `oidc_start`/`oidc_callback` routes above.
///
/// # Errors
///
/// Cannot fail on its own — every field is read from already-resolved,
/// in-memory state (`AppState::auth`, `Config::oidc_provider_name`), never
/// a database or network call.
pub async fn providers(State(state): State<AppState>) -> ApiResult<ApiJson<ProvidersResponse>> {
    // `ADM-12` D10: readiness, not mere configuration. A saved connection
    // that cannot be used (no `CONSOLE_PUBLIC_URL`, an unreachable
    // discovery, a refused address) reports no OIDC, so the sign-in page
    // shows no button that would fail; the Settings form says why.
    let runtime = crate::runtime_settings::sso_runtime(&state).await;
    let ready = runtime.is_ready();
    Ok(ApiJson(ProvidersResponse {
        oidc: ready,
        provider_name: ready.then(|| runtime.settings.provider_name.clone()),
    }))
}

/// `GET /api/auth/oidc/callback?code=&state=` query parameters. Both
/// `Option` — a missing one is a 401 (see [`oidc_callback`]), not a 400:
/// an `IdP` calling back with neither is indistinguishable from a bare
/// probe/replay of this URL, and the existing flow-cookie-driven 401 path
/// already covers it without a second error shape.
#[derive(Debug, Deserialize)]
pub struct OidcCallbackQuery {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: Option<String>,
}

/// The query parameter a successful callback appends to the landing path
/// when the sign-in had a caveat (`SEC-39`). The console maps its value to
/// the sentence it shows (`src/features/auth/sso-notice.tsx`); this service
/// never puts prose — or anything from the token — into the URL.
const OIDC_NOTICE_PARAM: &str = "ssoNotice";

/// Where a callback refused for `SEC-65` D2 sends the person: the sign-in
/// page with a fixed notice code (`email_not_verified`). The sentence lives
/// in the console (`src/features/auth/sso-notice.tsx` `NOTICES`); neither
/// the email nor the subject ever appears in this address.
const EMAIL_NOT_VERIFIED_LOCATION: &str = "/login?ssoNotice=email_not_verified";

/// `next` with `ssoNotice=<code>` appended, keeping any query `next`
/// already carried. `next` has already passed `is_safe_relative_path`
/// (inside [`oidc_start`], before it was written to the flow cookie), so
/// this only adds a fixed parameter name and an allowlisted code.
///
/// `is_safe_relative_path` allows a `#fragment`, so the parameter goes
/// before it: after the `#` it would be part of the fragment, which the
/// console's `useSearchParams` never reads (PR review, `SEC-39`).
fn path_with_notice(next: &str, code: &str) -> String {
    let (path, fragment) = next.find('#').map_or((next, ""), |at| next.split_at(at));
    let separator = if path.contains('?') { '&' } else { '?' };
    format!("{path}{separator}{OIDC_NOTICE_PARAM}={code}{fragment}")
}

/// The one field this handler reads from the token endpoint's response.
/// Deliberately not `#[serde(deny_unknown_fields)]`: an `IdP` is free to
/// also return `access_token`/`refresh_token`/`expires_in`/etc, none of
/// which this authorization-code + PKCE *login* flow (as opposed to a
/// resource-server bearer-token flow) has any use for — only the id token
/// is verified and turned into a session.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    id_token: String,
}

/// The one member a failed token response is read for, so a refusal is told
/// apart from an unreachable provider without ever carrying the provider's
/// text (`ADM-12` review round 1, S1; principle 4).
#[derive(Debug, Deserialize)]
struct TokenErrorResponse {
    error: String,
}

/// Why a code exchange did not produce an `id_token` (`ADM-12` review round
/// 1, S1). The provider's status and body are classified here and never
/// carried further.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExchangeFailure {
    /// The provider refused the client ID or secret: a 401/403, or its own
    /// `error` member saying `invalid_client`.
    Refused,
    /// The provider could not be reached at all.
    Unreachable,
    /// A status this deployment does not classify, or an answer that is not
    /// the expected shape.
    Other,
}

impl ExchangeFailure {
    /// The fixed sentence the test sign-in stores for this failure. Nothing
    /// the provider said is appended.
    const fn sentence(self) -> &'static str {
        match self {
            Self::Refused => "The provider refused the client ID or secret.",
            Self::Unreachable => "The identity provider could not be reached.",
            Self::Other => "The provider's answer to the sign-in was not usable.",
        }
    }
}

/// `GET /api/auth/oidc/callback?code=&state=` — the second half of the
/// login flow [`oidc_start`] began: exchange the authorization code for an
/// id token, verify it (signature, `iss`, `aud`, `exp`/`nbf`, algorithm
/// allowlist, and the `nonce` bound to this flow — all via
/// [`lakehouse_auth::OidcAuthenticator::authenticate_with_nonce`], Task
/// A3; none of that is reimplemented here), mint a brand-new session
/// (never reusing or upgrading any session the caller's browser might
/// already be carrying — that would be session fixation), and redirect to
/// the flow cookie's `next`.
///
/// The flow cookie is read exactly once, at the top of this function, and
/// cleared (`Max-Age=0`) in EVERY response this handler returns, success
/// or failure — a second request replaying the same query string always
/// then fails at the missing-cookie check below, which is what makes the
/// whole exchange single-use regardless of how the first attempt turned
/// out.
///
/// On success the redirect to `next` carries `ssoNotice=<code>` when the
/// sign-in had a caveat — today only Entra ID's group overage (`SEC-39`),
/// which also gets a `warn` line from
/// [`lakehouse_auth::OidcAuthenticator::authenticate_with_nonce`]. The
/// console maps the code to its own sentence; this handler adds no prose.
///
/// `SEC-65` D2: when the provider did not confirm the token's email and an
/// account already holds it, the answer is a redirect to the sign-in page
/// with `ssoNotice=email_not_verified`, not the generic 401 body — this is
/// the one refusal a person can act on. Nothing is created, no session is
/// minted, and the refusal was audited where it was decided
/// (`lakehouse_auth::oidc`'s `provision_user`). The code, like every other
/// notice, is fixed; the email and the subject never appear in the address.
///
/// # Errors
///
/// Returns 401 on a missing/undecodable/unparseable flow cookie, a
/// missing `code`/`state`, a `state` mismatch (compared in constant time
/// via [`lakehouse_auth::Secret::constant_time_eq`] — never `==`, which
/// would leak how many leading bytes matched through response timing), a
/// token-exchange HTTP failure, or a verification failure from
/// [`lakehouse_auth::OidcAuthenticator::authenticate_with_nonce`]. The one
/// exception is `AuthError::EmailNotVerified` (`SEC-65` D2), which redirects
/// to the sign-in page with a notice rather than answering 401 (see above).
/// Every other one of those renders the same generic 401 body: per AGENTS.md
/// principle 4 ("upstream error text never reaches a response"), this
/// handler never puts the `IdP`'s own error body, or which specific check
/// failed, into anything sent back to the browser — only a `tracing::warn!`
/// line records which branch fired, and even that carries no token/secret
/// value. Returns 503 if OIDC or its token endpoint is not configured, or
/// no Postgres pool is configured.
#[allow(
    clippy::too_many_lines,
    reason = "0045_session_oidc_mapped_roles.sql's fix threads mapped_role_names \
              from authenticate_with_nonce through to create_session, which pushed \
              this already-long, already-documented handler a few lines over the \
              limit -- splitting it would scatter one linear login flow across \
              functions for a threshold, not a real seam"
)]
pub async fn oidc_callback(
    State(state): State<AppState>,
    Query(query): Query<OidcCallbackQuery>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    // Built once and reused by every branch below (see the doc comment
    // above on why the cookie must be cleared unconditionally) rather than
    // being repeated at each return site, which could drift.
    let clear_cookie = || oidc_flow_cookie_header(state.config.is_dev, None);
    let unauthorized_and_clear = |mut resp: Response| {
        resp.headers_mut()
            .append(header::SET_COOKIE, clear_cookie());
        resp
    };
    let unauthorized =
        || unauthorized_and_clear(ApiRejection(ApiError::unauthorized()).into_response());
    let unavailable =
        |msg: &str| ApiRejection(ApiError::Unavailable(msg.to_owned())).into_response();
    let unavailable_and_clear = |msg: &str| unauthorized_and_clear(unavailable(msg));

    // `ADM-12` plan D8: if this request carries an SSO test flow cookie,
    // route directly to the draft verification handler BEFORE checking whether
    // live OIDC is configured on this deployment — the whole purpose of the
    // test flow is to test a draft configuration before saving it.
    if let Some(flow) = decode_flow_cookie(&headers)
        && let Some(test_run_id) = flow.test_run_id
    {
        return handle_oidc_test_callback(state, query, flow, test_run_id).await;
    }

    // Deployment-configuration presence is checked BEFORE the regular flow cookie
    // is processed — same order [`oidc_start`] already uses for the same
    // three settings (`auth.oidc`/token URL/redirect URI/client ID) plus
    // this handler's own `OIDC_TOKEN_URL`. This is a "is OIDC configured
    // on this deployment at all" question, independent of any particular
    // caller's cookie, so it fails closed to 503 (not 401) exactly like
    // [`oidc_start`] does when asked the same question — a 503 here is not
    // a "no credentials presented" 401/403 for [`crate::policy::POLICY_TABLE`]'s
    // `Policy::Public` purposes, and nothing has been read from `headers`
    // yet, so there is no cookie to clear on this path.
    let runtime = crate::runtime_settings::sso_runtime(&state).await;
    // R2-B1: a bearer-only deployment has an authenticator but cannot sign
    // anyone in; this route is a sign-in's second half, so it needs the
    // ready fact, not merely "an authenticator exists".
    let Some(auth) = runtime.sign_in_authenticator() else {
        return Ok(unavailable(
            runtime
                .not_ready_reason()
                .unwrap_or("OIDC is not configured on this deployment"),
        ));
    };
    let sso = runtime.settings.clone();
    let (Some(token_url), Some(redirect_uri)) =
        (sso.token_url.as_deref(), sso.redirect_uri.as_deref())
    else {
        return Ok(unavailable(
            "OIDC_TOKEN_URL/OIDC_REDIRECT_URI are not configured",
        ));
    };
    let Some(client_id) = sso.client_id.as_deref() else {
        return Ok(unavailable("OIDC_CLIENT_ID is not configured"));
    };

    // From here on, every failure is a caller-supplied-value problem (bad
    // cookie, bad state, failed exchange, failed verification), so it
    // fails closed to 401 with the flow cookie cleared — see this
    // function's doc comment.
    let Some(flow) = decode_flow_cookie(&headers) else {
        return Ok(unauthorized());
    };

    let (Some(code), Some(returned_state)) = (query.code, query.state) else {
        tracing::warn!("oidc callback: missing code or state");
        return Ok(unauthorized());
    };
    // Constant-time: `state` is the CSRF token binding this response to
    // the request `oidc_start` issued, and a variable-time `==` would leak
    // how many leading characters matched through response timing — the
    // exact defect this task exists to close.
    let state_matches =
        Secret::new(returned_state).constant_time_eq(&Secret::new(flow.state.clone()));
    if !state_matches {
        tracing::warn!("oidc callback: state mismatch");
        return Ok(unauthorized());
    }

    let client_secret = if sso.client_secret_set {
        Some(sso.client_secret.expose_secret())
    } else {
        state.config.oidc_client_secret.as_deref()
    };
    let Ok(id_token) = exchange_code_for_id_token(
        &runtime.http,
        token_url,
        redirect_uri,
        client_id,
        client_secret,
        &code,
        &flow.code_verifier,
    )
    .await
    else {
        return Ok(unauthorized());
    };

    let login = match auth
        .authenticate_with_nonce(&Secret::new(id_token), &flow.nonce)
        .await
    {
        Ok(login) => login,
        // `SEC-65` D2: the provider did not confirm the email and an
        // account holds it, so the sign-in is refused. The person is sent
        // back to the sign-in page with a fixed code instead of the
        // generic 401 body, because this refusal is something they can
        // act on ("ask an administrator"); nothing is created, no session
        // cookie is set, and the refusal itself was already audited where
        // it was decided (`provision_user`). The flow cookie is cleared
        // like every other answer from this handler.
        Err(AuthError::EmailNotVerified) => {
            let mut response = (StatusCode::FOUND, ()).into_response();
            response.headers_mut().insert(
                header::LOCATION,
                HeaderValue::from_static(EMAIL_NOT_VERIFIED_LOCATION),
            );
            response
                .headers_mut()
                .append(header::SET_COOKIE, clear_cookie());
            return Ok(response);
        }
        Err(_) => {
            tracing::warn!("oidc callback: id token verification failed");
            return Ok(unauthorized());
        }
    };
    let PrincipalId::User(user_id) = login.principal.id else {
        // `authenticate_with_nonce` only ever resolves a
        // `PrincipalId::User` (`resolve_principal` loads through
        // `app_user`) — this branch exists so the match stays exhaustive,
        // not because it is reachable in practice (same shape as
        // [`login`]'s identical branch).
        tracing::warn!("oidc callback: resolved principal was not a user");
        return Ok(unauthorized());
    };
    let Some(pool) = state.pg.as_deref() else {
        return Ok(unavailable_and_clear(
            "authentication unavailable: no Postgres pool is configured (set DATABASE_URL)",
        ));
    };

    // A fresh session — never an upgrade of any session the caller's
    // browser might already be carrying — closes the session-fixation gap
    // this task exists to close (see this function's doc comment). The
    // same `session::create_session` call [`login`] already uses, not a
    // parallel implementation, except this one also carries
    // `mapped_role_names` (`0045_session_oidc_mapped_roles.sql`) so the
    // OIDC-mapped permissions `authenticate_with_nonce` resolved for this
    // login keep applying on every request this session makes, not just
    // this one redirect.
    let token = session::create_session(
        pool,
        user_id,
        session::DEFAULT_SESSION_TTL,
        // ADM-14 D10: no address — the API sits behind the console's own
        // server and has no trusted source for the client's.
        None,
        session_user_agent(&headers).as_deref(),
        &login.mapped_role_names,
    )
    .await?;

    // audit_event row mirrors 0024_audit_event.sql's schema — `outcome = "executed"`
    // per `audit_event_outcome_check` (0024_audit_event.sql).
    // `resource_id` is intentionally `None` here: the session row's own
    // id is not yet known without a second `SELECT` keyed on the hash,
    // which this task does not add for a login-audit line that already
    // carries `principal_id`. Best-effort: a failed audit write is
    // logged and swallowed, never propagated, so the response on the
    // wire is always the underlying login's, not the audit's.
    let audit_event = oidc_login_audit_event(&login.principal, user_id);
    if let Err(err) = store_audit::insert(pool, audit_event).await {
        tracing::warn!(
            %err,
            action = "auth.oidc.login",
            user_id = %user_id,
            "failed to record oidc login audit event"
        );
    }

    let mut response = (StatusCode::FOUND, ()).into_response();
    // `flow.next` was already validated by `is_safe_relative_path` inside
    // `oidc_start` before it was ever written to the cookie (see
    // `OidcFlow`'s doc comment) — never re-derived from this request, so
    // there is nothing left to re-validate here. The fallback exists only
    // for the type-level possibility that a valid relative path is somehow
    // not a valid header value, not because a value that already passed
    // `is_safe_relative_path` is expected to fail this.
    //
    // `SEC-39`: a sign-in with a caveat says so on the page the user lands
    // on. The notice is a fixed code, never the token's claims.
    let location = match login.notice {
        Some(notice) => path_with_notice(&flow.next, notice.code()),
        None => flow.next,
    };
    response.headers_mut().insert(
        header::LOCATION,
        HeaderValue::from_str(&location).unwrap_or_else(|_| HeaderValue::from_static("/")),
    );
    response.headers_mut().append(
        header::SET_COOKIE,
        session_cookie_header(
            state.config.is_dev,
            token.expose(),
            session::DEFAULT_SESSION_TTL,
        ),
    );
    response
        .headers_mut()
        .append(header::SET_COOKIE, clear_cookie());
    Ok(response)
}

/// Read, base64url-decode, and JSON-parse the `lh_oidc_flow` cookie out of
/// `headers`, folding all three failure modes (no cookie, bad base64, bad
/// JSON) into one `None` — every caller treats them identically (401, per
/// [`oidc_callback`]'s doc comment), so there is no reason to distinguish
/// them past a `tracing::warn!` line naming which one fired.
fn decode_flow_cookie(headers: &HeaderMap) -> Option<OidcFlow> {
    let raw_cookie = oidc_flow_cookie(headers)?;
    let Ok(flow_json) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(&raw_cookie) else {
        tracing::warn!("oidc callback: flow cookie was not valid base64url");
        return None;
    };
    let Ok(flow) = serde_json::from_slice::<OidcFlow>(&flow_json) else {
        tracing::warn!("oidc callback: flow cookie did not decode to a valid flow");
        return None;
    };
    Some(flow)
}

/// POST the authorization code (and PKCE `code_verifier`) to `token_url`
/// through `http` — the client the connection was built with, so the
/// address that passed the allowlist check is the one dialled (`ADM-12`
/// D10) — and return the `id_token` from a successful response.
///
/// # Errors
///
/// Returns a classified [`ExchangeFailure`] (`ADM-12` review round 1, S1):
/// [`ExchangeFailure::Refused`] for a client-ID/secret refusal,
/// [`ExchangeFailure::Unreachable`] for a request that never completed, and
/// [`ExchangeFailure::Other`] otherwise. Never surfaces the `IdP`'s own
/// status/body to the caller or into anything but a `tracing::warn!` line
/// (AGENTS.md principle 4).
async fn exchange_code_for_id_token(
    http: &reqwest::Client,
    token_url: &str,
    redirect_uri: &str,
    client_id: &str,
    client_secret: Option<&str>,
    code: &str,
    code_verifier: &str,
) -> Result<String, ExchangeFailure> {
    let mut form = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("code_verifier", code_verifier),
        ("client_id", client_id),
    ];
    if let Some(secret) = client_secret {
        form.push(("client_secret", secret));
    }
    let response = http.post(token_url).form(&form).send().await.map_err(|_| {
        tracing::warn!("oidc: token exchange request failed");
        ExchangeFailure::Unreachable
    })?;
    if !response.status().is_success() {
        let status = response.status();
        // Only the `error` member is read, and only to tell a refusal from a
        // status this deployment does not classify. The member's value is
        // compared, never logged or returned.
        let invalid_client = response
            .json::<TokenErrorResponse>()
            .await
            .is_ok_and(|body| body.error == "invalid_client");
        tracing::warn!("oidc: the token endpoint did not accept the code exchange");
        return Err(
            if invalid_client
                || status == StatusCode::UNAUTHORIZED
                || status == StatusCode::FORBIDDEN
            {
                ExchangeFailure::Refused
            } else {
                ExchangeFailure::Other
            },
        );
    }
    response
        .json::<TokenResponse>()
        .await
        .map(|response| response.id_token)
        .map_err(|_| {
            tracing::warn!("oidc: token endpoint response was not the expected shape");
            ExchangeFailure::Other
        })
}

/// Read the `lh_oidc_flow` cookie's raw (still base64url-encoded) value
/// out of the request's `Cookie` header, or `None` if it is absent —
/// mirrors `crate::auth::session_cookie_from_headers`'s parsing shape for
/// the session cookie, but is local to this module since `OIDC_FLOW_COOKIE_NAME`
/// is private to it.
fn oidc_flow_cookie(headers: &HeaderMap) -> Option<String> {
    let header = headers.get(header::COOKIE)?.to_str().ok()?;
    header.split(';').find_map(|pair| {
        let (name, value) = pair.trim().split_once('=')?;
        (name == OIDC_FLOW_COOKIE_NAME).then(|| value.to_owned())
    })
}

/// Build the `NewAuditEvent` for a successful OIDC login — extracted as
/// a pure, unit-testable helper so the field-shape lives next to the
/// call site (rather than a brittle source-text grep), mirroring
/// `routes::identity::rotate_service_identity_audit_event`. The audit
/// row is `outcome = "executed"` because login is a successful write —
/// the schema's `audit_event_outcome_check` allows it. `principal_kind`
/// comes from `Principal::kind_for_audit`, not from `principal.provider`
/// (the two are different vocabularies — see `Principal::kind_for_audit`'s
/// own doc comment).
fn oidc_login_audit_event(principal: &Principal, user_id: Uuid) -> NewAuditEvent {
    NewAuditEvent {
        action: "auth.oidc.login".to_owned(),
        resource_kind: Some("session".to_owned()),
        resource_id: None,
        principal_id: Some(user_id.to_string()),
        principal_kind: Some(principal.kind_for_audit().to_owned()),
        actor_label: Some(principal.display_name.clone()),
        outcome: "executed".to_owned(),
        detail: Some(format!("provider={}", principal.provider)),
        ..Default::default()
    }
}

/// Store a test run's result and record the audit row (`ADM-12` D8): one
/// place, so every failure path stores the same shape and none can forget
/// the audit. The sentence in `result.error` is fixed by the caller; nothing
/// the provider said is in it.
async fn store_sso_test_result(
    pool: &PgPool,
    test_run_id: Uuid,
    created_by: Uuid,
    result: serde_json::Value,
) {
    let _ = lakehouse_store::sso_settings::update_test_run_result(pool, test_run_id, &result).await;
    let outcome = if result.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
        "executed"
    } else {
        "failed"
    };
    audit_sso_test(pool, created_by, outcome, &result).await;
}

/// The second half of a test sign-in (`ADM-12` D8): exchange the code and
/// verify the ID token against the **draft's own** settings, store what the
/// provider sent, and create no user, no identity and no session.
///
/// `ADM-12` review round 1, B2: every address and every setting comes from
/// the run's stored draft, through the same builder the live path uses — the
/// token endpoint, the key set and the authenticate address never come from
/// the live connection, so a draft's secret is never sent to a provider the
/// draft did not name. S4: the client secret is the one sealed into the run,
/// never `OIDC_CLIENT_SECRET`.
///
/// # Errors
///
/// Returns 401 for a missing/expired run, a missing `code`/`state`, or a
/// `state` mismatch, and 503 without a Postgres pool. Every provider-side
/// failure is stored as a fixed sentence and answered with the redirect back
/// to the Settings page (D8).
#[allow(
    clippy::too_many_lines,
    reason = "one linear test-sign-in flow: every failure stores one fixed sentence and returns the same redirect (ADM-12 D8)"
)]
async fn handle_oidc_test_callback(
    state: AppState,
    query: OidcCallbackQuery,
    flow: OidcFlow,
    test_run_id: Uuid,
) -> ApiResult<Response> {
    let clear_cookie = || oidc_flow_cookie_header(state.config.is_dev, None);
    let unauthorized_and_clear = |mut resp: Response| {
        resp.headers_mut()
            .append(header::SET_COOKIE, clear_cookie());
        resp
    };
    let unauthorized = || {
        unauthorized_and_clear(crate::error::ApiRejection(ApiError::unauthorized()).into_response())
    };
    let unavailable_and_clear = |msg: &str| {
        unauthorized_and_clear(
            crate::error::ApiRejection(ApiError::Unavailable(msg.to_owned())).into_response(),
        )
    };

    let Some(pool) = state.pg.as_deref() else {
        return Ok(unavailable_and_clear(
            "authentication unavailable: no Postgres pool is configured (set DATABASE_URL)",
        ));
    };

    let run = match lakehouse_store::sso_settings::get_test_run(pool, test_run_id).await {
        Ok(Some(r)) if r.expires_at > time::OffsetDateTime::now_utc() => r,
        _ => return Ok(unauthorized()),
    };
    let created_by = run.created_by;
    let draft = run.draft;
    let sealed_secret = run.draft_secret;

    let landing_target = format!("/admin/settings?section=sign-in&ssoTest={test_run_id}");
    let mut redirect_resp = (StatusCode::FOUND, ()).into_response();
    if let Ok(loc) = HeaderValue::from_str(&landing_target) {
        redirect_resp.headers_mut().insert(header::LOCATION, loc);
    }
    redirect_resp
        .headers_mut()
        .append(header::SET_COOKIE, clear_cookie());

    let (Some(code), Some(returned_state)) = (query.code, query.state) else {
        tracing::warn!("oidc test callback: missing code or state");
        store_sso_test_result(
            pool,
            test_run_id,
            created_by,
            serde_json::json!({
                "ok": false,
                "error": "The provider returned no authorization code or state.",
            }),
        )
        .await;
        return Ok(redirect_resp);
    };

    let state_matches =
        Secret::new(returned_state).constant_time_eq(&Secret::new(flow.state.clone()));
    if !state_matches {
        tracing::warn!("oidc test callback: state mismatch");
        store_sso_test_result(
            pool,
            test_run_id,
            created_by,
            serde_json::json!({
                "ok": false,
                "error": "The sign-in answer did not match the request this deployment started.",
            }),
        )
        .await;
        return Ok(redirect_resp);
    }

    // B2: the draft's own settings, and the draft's own discovery document.
    // R2-B1: the draft must be able to sign someone in, so this takes the
    // ready fact — a draft whose discovery worked but which has no return
    // address cannot test a sign-in.
    let runtime = crate::runtime_settings::sso_runtime_for_settings(
        &state,
        crate::runtime_settings::draft_sso_settings(&state.config, &draft),
    )
    .await;
    let Some(authenticator) = runtime.sign_in_authenticator() else {
        let reason = runtime
            .not_ready_reason()
            .unwrap_or("The draft connection cannot be used to test a sign-in.");
        store_sso_test_result(
            pool,
            test_run_id,
            created_by,
            serde_json::json!({ "ok": false, "error": reason }),
        )
        .await;
        return Ok(redirect_resp);
    };
    let (Some(token_url), Some(redirect_uri), Some(client_id)) = (
        runtime.settings.token_url.as_deref(),
        runtime.settings.redirect_uri.as_deref(),
        runtime.settings.client_id.as_deref(),
    ) else {
        store_sso_test_result(
            pool,
            test_run_id,
            created_by,
            serde_json::json!({
                "ok": false,
                "error": "The draft is missing an address a sign-in needs.",
            }),
        )
        .await;
        return Ok(redirect_resp);
    };

    // S4: this run uses ONLY the secret sealed into it when it was created.
    // No `OIDC_CLIENT_SECRET`, and no live connection's secret.
    let client_secret = match sealed_secret.as_deref() {
        None => None,
        Some(sealed) => {
            let opened = state
                .connector_secret_store
                .keyring()
                .ok()
                .and_then(|ring| {
                    ring.open(crate::runtime_settings::SSO_CLIENT_SECRET_SEAL_NAME, sealed)
                        .ok()
                })
                .and_then(|opened| String::from_utf8(opened.plaintext).ok());
            let Some(opened) = opened else {
                tracing::warn!("oidc test callback: the run's sealed secret could not be opened");
                store_sso_test_result(
                    pool,
                    test_run_id,
                    created_by,
                    serde_json::json!({
                        "ok": false,
                        "error": "The secret saved with this test sign-in could not be opened.",
                    }),
                )
                .await;
                return Ok(redirect_resp);
            };
            Some(opened)
        }
    };

    let id_token = match exchange_code_for_id_token(
        &runtime.http,
        token_url,
        redirect_uri,
        client_id,
        client_secret.as_deref(),
        &code,
        &flow.code_verifier,
    )
    .await
    {
        Ok(id_token) => id_token,
        Err(failure) => {
            store_sso_test_result(
                pool,
                test_run_id,
                created_by,
                serde_json::json!({ "ok": false, "error": failure.sentence() }),
            )
            .await;
            return Ok(redirect_resp);
        }
    };

    match authenticator
        .validate_for_test(&Secret::new(id_token), &flow.nonce)
        .await
    {
        Ok(claims) => {
            // S2: `emailVerified` and `overage` are part of what an
            // administrator sees (`SEC-65` will need the former).
            store_sso_test_result(
                pool,
                test_run_id,
                created_by,
                serde_json::json!({
                    "ok": true,
                    "subject": claims.subject,
                    "email": claims.email,
                    "emailVerified": claims.email_verified,
                    "groups": claims.groups,
                    "mappedRoles": claims.mapped_roles,
                    "overage": claims.overage,
                }),
            )
            .await;
        }
        Err(err) => {
            tracing::warn!(error = %err, "oidc test callback: id token validation failed");
            store_sso_test_result(
                pool,
                test_run_id,
                created_by,
                serde_json::json!({
                    "ok": false,
                    "error": "The ID token could not be verified.",
                }),
            )
            .await;
        }
    }

    Ok(redirect_resp)
}

pub(crate) async fn audit_sso_test(
    pool: &PgPool,
    created_by: Uuid,
    outcome: &str,
    result: &serde_json::Value,
) {
    let event = NewAuditEvent {
        principal_id: Some(created_by.to_string()),
        principal_kind: Some("user".to_owned()),
        actor_label: None,
        action: "settings.sso.test".to_owned(),
        resource_kind: Some("settings".to_owned()),
        resource_id: Some("sso".to_owned()),
        args: Some(result.clone()),
        outcome: outcome.to_owned(),
        detail: None,
        run_id: None,
        approval_id: None,
        session_id: None,
    };
    if let Err(err) = store_audit::insert(pool, event).await {
        tracing::warn!(%err, action = "settings.sso.test", "failed to record the sso test audit event");
    }
}

/// Build the `NewAuditEvent` for a successful session revoke — same
/// pattern as [`oidc_login_audit_event`]: pure, unit-testable, mirrors
/// `routes::identity::rotate_service_identity_audit_event`. `resource_id`
/// is the request path's `{id}` (the session row's UUID), never the
/// caller's bearer token.
fn session_revoke_audit_event(principal: &Principal, session_id: Uuid) -> NewAuditEvent {
    NewAuditEvent {
        action: "auth.session.revoke".to_owned(),
        resource_kind: Some("session".to_owned()),
        resource_id: Some(session_id.to_string()),
        principal_id: Some(principal.id.uuid().to_string()),
        principal_kind: Some(principal.kind_for_audit().to_owned()),
        actor_label: Some(principal.display_name.clone()),
        outcome: "executed".to_owned(),
        ..Default::default()
    }
}

/// Response body for `GET /api/auth/password-rules` (ADM-11 D4, moved above tests for ADM-11 review 1 B4).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PasswordRulesResponse {
    /// Minimum password length.
    pub min_length: usize,
    /// Maximum password length.
    pub max_length: usize,
}

/// `GET /api/auth/password-rules` — public endpoint returning password length constraints (ADM-11 D4).
///
/// ADM-11 review 1 B4: placed above `#[cfg(test)]` so the SEC-11 scan does not skip it.
///
/// # Errors
///
/// This handler never fails and returns `Ok`.
pub async fn password_rules() -> ApiResult<ApiJson<PasswordRulesResponse>> {
    Ok(ApiJson(PasswordRulesResponse {
        min_length: password::MIN_PASSWORD_LENGTH,
        max_length: password::MAX_PASSWORD_LENGTH,
    }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use tower::ServiceExt;

    use super::*;
    use crate::config::Config;

    /// Same idiom `routes::gold::tests::state_without_pool` uses: an
    /// unreachable-but-well-formed `DATABASE_URL` (never actually
    /// dialled — `lakehouse_store::connect_lazy` performs no I/O, see
    /// its own doc comment) plus every `OIDC_*` flow-config var
    /// `oidc_start` needs, so `AppState::new` boots with `auth.oidc` populated
    /// without a real Postgres instance or network call.
    fn state_with_oidc_flow_config() -> AppState {
        let mut env = HashMap::new();
        env.insert(
            "DATABASE_URL".to_owned(),
            "postgres://user:pass@127.0.0.1:1/nonexistent_db_xyz".to_owned(),
        );
        env.insert("OIDC_ISSUER".to_owned(), "http://idp.invalid".to_owned());
        env.insert("OIDC_CLIENT_ID".to_owned(), "lakehouse-console".to_owned());
        env.insert(
            "OIDC_AUTHORIZE_URL".to_owned(),
            "http://idp.invalid/authorize".to_owned(),
        );
        env.insert(
            "OIDC_TOKEN_URL".to_owned(),
            "http://idp.invalid/token".to_owned(),
        );
        env.insert(
            "OIDC_REDIRECT_URI".to_owned(),
            "https://lake.invalid/api/auth/oidc/callback".to_owned(),
        );
        let config = Config::from_map(&env).expect("a valid test Config");
        AppState::new(config)
    }

    /// Base64url-decode + JSON-parse a `Set-Cookie: lh_oidc_flow=...`
    /// header value's cookie payload, for tests that need to inspect the
    /// flow it actually carries (not just that a cookie was set).
    fn decode_flow_cookie(set_cookie: &str) -> OidcFlow {
        let value = set_cookie
            .strip_prefix(&format!("{OIDC_FLOW_COOKIE_NAME}="))
            .expect("cookie header starts with the flow cookie name")
            .split(';')
            .next()
            .expect("at least one cookie-attribute segment");
        let json = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(value)
            .expect("valid base64url");
        serde_json::from_slice(&json).expect("valid OidcFlow JSON")
    }

    /// `QA LH-SEC-5-01` (`ADM-14` D9): the browser is recorded as it named
    /// itself — trimmed, and cut to a length the column and the page can
    /// carry.
    #[test]
    fn the_session_user_agent_is_the_header_trimmed_and_cut_to_512_characters() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::USER_AGENT,
            HeaderValue::from_static("QA-Browser/1.0"),
        );
        assert_eq!(
            session_user_agent(&headers).as_deref(),
            Some("QA-Browser/1.0")
        );

        headers.insert(
            header::USER_AGENT,
            HeaderValue::from_static("   QA-Browser/1.0   "),
        );
        assert_eq!(
            session_user_agent(&headers).as_deref(),
            Some("QA-Browser/1.0"),
            "surrounding whitespace is trimmed"
        );

        let long = "A".repeat(700);
        headers.insert(
            header::USER_AGENT,
            HeaderValue::from_str(&long).expect("a 700-character header value"),
        );
        let recorded = session_user_agent(&headers).expect("a long header is recorded, cut short");
        assert_eq!(recorded.chars().count(), 512);
        assert!(recorded.chars().all(|c| c == 'A'));
    }

    /// `QA LH-SEC-5-01` (`ADM-14` D9): nothing is recorded for a header that
    /// is absent, empty, or not plain ASCII — a value the page would show as
    /// fact has to be the client's own text, not a lossy conversion.
    #[test]
    fn the_session_user_agent_is_nothing_for_an_absent_empty_or_non_ascii_header() {
        assert_eq!(session_user_agent(&HeaderMap::new()), None);

        let mut headers = HeaderMap::new();
        headers.insert(header::USER_AGENT, HeaderValue::from_static("   "));
        assert_eq!(
            session_user_agent(&headers),
            None,
            "whitespace only is empty"
        );

        headers.insert(
            header::USER_AGENT,
            HeaderValue::from_bytes("QA-Browser/1.0 é".as_bytes())
                .expect("obs-text is a legal header value"),
        );
        assert_eq!(
            session_user_agent(&headers),
            None,
            "not plain ASCII is not recorded"
        );
    }

    #[tokio::test]
    async fn oidc_start_redirects_to_the_configured_authorize_url_and_sets_a_flow_cookie() {
        let state = state_with_oidc_flow_config();
        let app = crate::routes::router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/auth/oidc/start")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FOUND);
        let location = response
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(location.starts_with("http://idp.invalid/authorize?"));
        assert!(location.contains("code_challenge_method=S256"));
        assert!(location.contains("response_type=code"));
        let set_cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(set_cookie.starts_with("lh_oidc_flow="));
        assert!(set_cookie.contains("HttpOnly"));
        assert!(set_cookie.contains("SameSite=Lax"));
    }

    #[tokio::test]
    async fn oidc_start_rejects_every_off_origin_or_ambiguous_next_shape() {
        // These four off-origin/ambiguous `next` shapes, plus their
        // percent-encoded forms, must all be refused; each is asserted
        // individually rather than folded into one loop, so a future
        // regression's failure names exactly which shape stopped being
        // refused.
        let cases = [
            "https://attacker.invalid/steal", // absolute URL, wrong origin
            "//evil.invalid/steal",           // scheme-relative absolute URL
            "/\\evil.invalid/steal",          // leading backslash — browsers normalize \ toward /
            "/legit/path\\/evil.invalid",     // an embedded backslash anywhere, not just leading
            "%2F%2Fevil.invalid",             // percent-encoded "//evil.invalid"
            "%2Fpath%5Cevil.invalid",         // percent-encoded "/path\evil.invalid"
        ];
        for raw_next in cases {
            let state = state_with_oidc_flow_config();
            let app = crate::routes::router(state);
            let uri = format!("/api/auth/oidc/start?next={raw_next}");
            let response = app
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::FOUND,
                "case {raw_next:?} must still redirect to the IdP"
            );
            let set_cookie = response
                .headers()
                .get(header::SET_COOKIE)
                .unwrap()
                .to_str()
                .unwrap();
            // The rejected `next` must never appear in the flow cookie at
            // all — it is discarded in favor of the default "/", not
            // re-encoded and forwarded, which would just move the
            // open-redirect surface into the cookie instead of the header.
            assert!(
                !set_cookie.contains("evil.invalid") && !set_cookie.contains("attacker.invalid"),
                "case {raw_next:?} leaked into the flow cookie: {set_cookie}"
            );
        }
    }

    #[tokio::test]
    async fn oidc_start_accepts_a_genuine_same_origin_relative_next() {
        let state = state_with_oidc_flow_config();
        let app = crate::routes::router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/auth/oidc/start?next=/dashboards/main")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FOUND);
        let set_cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();
        let decoded = decode_flow_cookie(set_cookie);
        assert_eq!(decoded.next, "/dashboards/main");
    }

    #[test]
    fn path_with_notice_keeps_the_query_and_puts_the_notice_before_a_fragment() {
        // PR review (`SEC-39`): `next` may carry a query and a fragment;
        // the notice joins the query and never lands inside the fragment.
        assert_eq!(
            path_with_notice("/", "group_overage"),
            "/?ssoNotice=group_overage"
        );
        assert_eq!(
            path_with_notice("/dashboards?tab=1", "group_overage"),
            "/dashboards?tab=1&ssoNotice=group_overage"
        );
        assert_eq!(
            path_with_notice("/dashboards#chart-2", "group_overage"),
            "/dashboards?ssoNotice=group_overage#chart-2"
        );
        assert_eq!(
            path_with_notice("/d?tab=1#x?y", "group_overage"),
            "/d?tab=1&ssoNotice=group_overage#x?y"
        );
    }

    #[test]
    fn is_safe_relative_path_rejects_every_p3_named_shape() {
        // Unit-level pin, independent of the full HTTP round trip above —
        // this is the function the four named off-origin/ambiguous shapes
        // above (plus percent-encoded forms, which axum has already
        // decoded by the time this function runs) must refuse.
        for bad in [
            "https://evil.invalid",
            "//evil.invalid",
            "/\\evil.invalid",
            "/legit\\evil.invalid",
            "evil.invalid", // no leading '/' at all
            "/login",       // never bounce back into the login page itself
            "/login/sso",
        ] {
            assert!(!is_safe_relative_path(bad), "{bad:?} must be rejected");
        }
        for good in ["/", "/dashboards", "/dashboards/main?tab=history"] {
            assert!(is_safe_relative_path(good), "{good:?} must be accepted");
        }
    }

    // ── `GET /api/auth/oidc/callback` tests, written test-first
    // (before `oidc_callback` exists at all). ──────────────────────

    #[tokio::test]
    async fn oidc_callback_rejects_a_state_mismatch_and_clears_the_flow_cookie() {
        let state = state_with_oidc_flow_config();
        let app = crate::routes::router(state);
        let cookie = oidc_flow_cookie_header(
            true,
            Some(&OidcFlow {
                state: "state-from-start".to_owned(),
                nonce: "nonce-1".to_owned(),
                code_verifier: "verifier-1".to_owned(),
                next: "/dashboards".to_owned(),
                test_run_id: None,
            }),
        );
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/auth/oidc/callback?code=abc&state=state-does-not-match")
                    .header(header::COOKIE, cookie.to_str().unwrap())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let cleared = response
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(cleared.contains("Max-Age=0"));
    }

    #[tokio::test]
    async fn oidc_callback_rejects_a_missing_flow_cookie() {
        let state = state_with_oidc_flow_config();
        let app = crate::routes::router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/auth/oidc/callback?code=abc&state=whatever")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    // ── The one wiremock-backed round trip: matching state/nonce all the
    // way through a real (mock) token exchange and id-token verification.
    // This is the one test in this repository, short of a live compose
    // stack, that exercises `oidc_callback`'s token-exchange HTTP call end
    // to end — Phase F's `g2`/Phase A's `g4` extension cover the real
    // thing against `ops/oidc-mock`. ──────────────────────────────────────

    fn b64url(bytes: &[u8]) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    }

    /// A freshly generated RSA keypair plus the JWK describing its public
    /// half — local to this module rather than reused from
    /// `lakehouse-auth/tests/oidc.rs`, since that harness lives in a
    /// different crate's integration-test binary and isn't a library this
    /// crate can depend on.
    struct TestKey {
        kid: String,
        encoding_key: jsonwebtoken::EncodingKey,
        jwk: jsonwebtoken::jwk::Jwk,
    }

    impl TestKey {
        fn generate(kid: &str) -> Self {
            use rsa::pkcs1::{EncodeRsaPrivateKey, LineEnding};
            use rsa::traits::PublicKeyParts;

            // `rsa` 0.9 still requires a `rand_core` 0.6 `CryptoRngCore`,
            // one major behind this workspace's `rand` 0.10 (#9) — use
            // `rsa`'s own re-exported `OsRng` here rather than `rand::rng()`,
            // which no longer implements that trait.
            let mut rng = rsa::rand_core::OsRng;
            let private_key = rsa::RsaPrivateKey::new(&mut rng, 2048).expect("keygen");
            let public_key = private_key.to_public_key();
            let pem = private_key.to_pkcs1_pem(LineEnding::LF).expect("pkcs1 pem");
            let encoding_key =
                jsonwebtoken::EncodingKey::from_rsa_pem(pem.as_bytes()).expect("encoding key");

            let jwk = jsonwebtoken::jwk::Jwk {
                common: jsonwebtoken::jwk::CommonParameters {
                    key_id: Some(kid.to_owned()),
                    ..Default::default()
                },
                algorithm: jsonwebtoken::jwk::AlgorithmParameters::RSA(
                    jsonwebtoken::jwk::RSAKeyParameters {
                        key_type: jsonwebtoken::jwk::RSAKeyType::RSA,
                        n: b64url(&public_key.n().to_bytes_be()),
                        e: b64url(&public_key.e().to_bytes_be()),
                    },
                ),
            };

            Self {
                kid: kid.to_owned(),
                encoding_key,
                jwk,
            }
        }
    }

    #[derive(Serialize)]
    struct IdTokenClaims<'a> {
        sub: &'a str,
        iss: &'a str,
        aud: &'a str,
        exp: i64,
        nonce: &'a str,
        /// The email claim, when the test sign-in result is about it.
        #[serde(skip_serializing_if = "Option::is_none")]
        email: Option<&'a str>,
        /// The provider's `email_verified` claim (`ADM-12` D8/S2).
        #[serde(rename = "email_verified", skip_serializing_if = "Option::is_none")]
        email_verified: Option<bool>,
        /// The groups claim the configured `groupsClaim` names.
        #[serde(skip_serializing_if = "Option::is_none")]
        groups: Option<Vec<&'a str>>,
        /// Entra ID's overage marker (`SEC-39`): names each claim whose
        /// values did not fit in the token, so the claim itself is absent.
        /// `None` for every ordinary test token.
        #[serde(rename = "_claim_names", skip_serializing_if = "Option::is_none")]
        claim_names: Option<serde_json::Value>,
    }

    // ── `ADM-12` review round 1, B1 + D10: a connection saved in Settings,
    // with no `OIDC_*` variable at all, signs someone in end to end ────────
    //
    // The provider is reached over `https` (`wiremock` through
    // `tls_proxy::HttpsFront`): a console-set issuer must be `https://` (D6)
    // and a test must not weaken that check to reach a plaintext mock. The
    // test supplies the client that trusts the CA the front minted
    // (`AppState::sso_http_override`, compiled only for this crate's tests);
    // every address check still runs, and only the pins are replaced.

    /// A reachable `https` mock OIDC provider (`ADM-12` review round 1, B1):
    /// `wiremock` behind [`crate::tls_proxy::HttpsFront`], with a discovery
    /// document naming the authorize, token and key-set addresses at the
    /// same `https` origin, and a client that trusts the CA the front
    /// minted. Every address check still runs against the real
    /// `validate`/`resolve_checked` path.
    async fn https_mock_provider() -> (
        wiremock::MockServer,
        crate::tls_proxy::HttpsFront,
        String,
        reqwest::Client,
    ) {
        let server = wiremock::MockServer::start().await;
        let plain_port: u16 = url::Url::parse(&server.uri())
            .expect("the mock's URI parses")
            .port()
            .expect("the mock listens on a port");
        let front = crate::tls_proxy::HttpsFront::start("127.0.0.1", plain_port, "127.0.0.1").await;
        let issuer = format!("https://127.0.0.1:{}", front.port());
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path(
                "/.well-known/openid-configuration",
            ))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "issuer": issuer,
                    "authorization_endpoint": format!("{issuer}/authorize"),
                    "token_endpoint": format!("{issuer}/token"),
                    "jwks_uri": format!("{issuer}/.well-known/jwks.json"),
                })),
            )
            .mount(&server)
            .await;
        let client = reqwest::Client::builder()
            .add_root_certificate(
                reqwest::Certificate::from_pem(front.ca_pem().as_bytes())
                    .expect("the front's CA as a certificate"),
            )
            .build()
            .expect("a client trusting the test CA");
        (server, front, issuer, client)
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_connection_saved_in_settings_signs_in_with_no_oidc_env(
        pool: sqlx::PgPool,
    ) -> sqlx::Result<()> {
        use lakehouse_test_support as _;

        let (server, _front, issuer, http) = https_mock_provider().await;
        let key = TestKey::generate("kid-1");
        mount_jwks(&server, &key).await;

        // No `OIDC_*` variable at all: the connection comes from `PUT`, the
        // return address from `CONSOLE_PUBLIC_URL` (D10).
        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), database_url_for(&pool));
        env.insert(
            "CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS".to_owned(),
            "true".to_owned(),
        );
        env.insert(
            "CONSOLE_PUBLIC_URL".to_owned(),
            "https://lakehouse.test".to_owned(),
        );
        // A throwaway installation key, so the console may seal a client
        // secret (base64 of 32 bytes of 0x2a — not a secret).
        env.insert("CONNECTOR_SECRETS_KEY".to_owned(), {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD.encode([0x2a_u8; 32])
        });
        let mut state = AppState::new(Config::from_map(&env).expect("a valid test Config"));
        state.sso_http_override = Some(http);
        let app = crate::routes::router(state.clone());

        // The seeded Platform Admin holds `*:*`, so it may save SSO (D5).
        let (admin_id,): (Uuid,) = sqlx::query_as("SELECT id FROM app_user WHERE email = $1")
            .bind("fajar@meridian.example")
            .fetch_one(&pool)
            .await?;
        let session_token = lakehouse_auth::session::create_session(
            &pool,
            admin_id,
            lakehouse_auth::session::DEFAULT_SESSION_TTL,
            None,
            None,
            &[] as &[String],
        )
        .await
        .expect("a session for the seeded admin");
        let cookie = format!("{}={}", SESSION_COOKIE_NAME, session_token.expose());

        let saved = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/settings/sso")
                    .header(header::COOKIE, &cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "issuer": issuer,
                            "clientId": "lakehouse-console",
                            "clientSecret": "test-secret",
                            "providerName": "mock",
                            "jitProvisioning": true,
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(saved.status(), StatusCode::OK);
        let saved_body = body_json(saved).await;
        assert_eq!(saved_body["ready"], true, "{saved_body}");

        // The sign-in page is told there is a provider.
        let providers = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/providers")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let providers_body = body_json(providers).await;
        assert_eq!(providers_body["oidc"], true, "{providers_body}");

        // 1. The sign-in starts at the discovery document's authorization
        // endpoint, and returns to `CONSOLE_PUBLIC_URL`'s callback.
        let started = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/oidc/start?next=/dashboards")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(started.status(), StatusCode::FOUND);
        let location = started
            .headers()
            .get(header::LOCATION)
            .expect("a redirect target")
            .to_str()
            .expect("an ASCII location")
            .to_owned();
        assert!(
            location.starts_with(&format!("{issuer}/authorize?")),
            "the authorize address comes from discovery: {location}"
        );
        assert!(
            location.contains(
                "redirect_uri=https%3A%2F%2Flakehouse.test%2Fapi%2Fauth%2Foidc%2Fcallback"
            ),
            "the return address comes from CONSOLE_PUBLIC_URL: {location}"
        );
        let flow_cookie = started
            .headers()
            .get(header::SET_COOKIE)
            .expect("the flow cookie")
            .to_str()
            .expect("an ASCII cookie")
            .to_owned();
        let flow = decode_flow_cookie(&flow_cookie);

        // 2. The provider answers the code exchange at the discovery
        // document's token endpoint.
        let id_token = sign_id_token(
            &IdTokenClaims {
                sub: "user-saved-in-settings",
                iss: &issuer,
                aud: "lakehouse-console",
                exp: time::OffsetDateTime::now_utc().unix_timestamp() + 3600,
                nonce: &flow.nonce,
                email: None,
                email_verified: None,
                groups: None,
                claim_names: None,
            },
            &key,
        );
        mount_token_endpoint(&server, &id_token).await;

        let callback = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/auth/oidc/callback?code=code-1&state={}",
                        flow.state
                    ))
                    .header(
                        header::COOKIE,
                        flow_cookie.split(';').next().unwrap_or_default(),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(callback.status(), StatusCode::FOUND);
        assert_eq!(
            callback
                .headers()
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok()),
            Some("/dashboards")
        );
        assert!(
            callback
                .headers()
                .get_all(header::SET_COOKIE)
                .iter()
                .any(|value| value.to_str().is_ok_and(
                    |text| text.starts_with("lh_session=") && !text.contains("Max-Age=0")
                )),
            "a session is created"
        );

        // 3. JIT provisioning created the account for the new subject.
        let (identities,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM auth_identity \
             WHERE provider = 'oidc:mock' AND external_subject = 'user-saved-in-settings'",
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(identities, 1, "the sign-in provisions the new subject");
        Ok(())
    }

    // ── `ADM-12` review round 1, B2/S1/S2/S4: the test sign-in uses the
    // draft's own provider, says what failed, and creates nothing ─────────

    /// A test sign-in against a draft is verified with the draft's own
    /// provider, reports `emailVerified` and `overage`, and creates no user,
    /// identity or session (D8; review round 1, S2).
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_test_sign_in_reports_the_draft_providers_claims_and_creates_nothing(
        pool: sqlx::PgPool,
    ) -> sqlx::Result<()> {
        use lakehouse_test_support as _;

        let (server, _front, issuer, http) = https_mock_provider().await;
        let key = TestKey::generate("kid-1");
        mount_jwks(&server, &key).await;

        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), database_url_for(&pool));
        env.insert(
            "CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS".to_owned(),
            "true".to_owned(),
        );
        env.insert(
            "CONSOLE_PUBLIC_URL".to_owned(),
            "https://lakehouse.test".to_owned(),
        );
        env.insert("CONNECTOR_SECRETS_KEY".to_owned(), {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD.encode([0x2a_u8; 32])
        });
        let mut state = AppState::new(Config::from_map(&env).expect("a valid test Config"));
        state.sso_http_override = Some(http);
        let admin = seeded_admin_id(&pool).await?;

        let users_before: i64 = sqlx::query_scalar("SELECT count(*) FROM app_user")
            .fetch_one(&pool)
            .await?;
        let identities_before: i64 = sqlx::query_scalar("SELECT count(*) FROM auth_identity")
            .fetch_one(&pool)
            .await?;
        let sessions_before: i64 = sqlx::query_scalar("SELECT count(*) FROM session")
            .fetch_one(&pool)
            .await?;

        let (flow_cookie, run_id) = start_sso_test(
            &state,
            admin,
            serde_json::json!({
                "issuer": issuer,
                "clientId": "draft-client",
                "clientSecret": "draft-secret",
                "providerName": "draft-provider",
                "groupsClaim": "groups",
                "roleMap": [{"group": "admins", "role": "Platform Admin"}],
                "jitProvisioning": true,
            }),
        )
        .await;
        let flow = decode_flow_cookie(&flow_cookie);

        let id_token = sign_id_token(
            &IdTokenClaims {
                sub: "draft-subject-1",
                iss: &issuer,
                aud: "draft-client",
                exp: time::OffsetDateTime::now_utc().unix_timestamp() + 3600,
                nonce: &flow.nonce,
                email: Some("draft@example.com"),
                email_verified: Some(true),
                groups: Some(vec!["admins"]),
                claim_names: None,
            },
            &key,
        );
        mount_token_endpoint(&server, &id_token).await;

        let response = finish_sso_test(&state, &flow_cookie, &flow, "code-1").await;
        assert_eq!(response.status(), StatusCode::FOUND);

        let run = lakehouse_store::sso_settings::get_test_run(&pool, run_id)
            .await
            .unwrap()
            .expect("the run exists");
        let result = run.result.expect("the callback stored a result");
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["subject"], "draft-subject-1");
        assert_eq!(result["email"], "draft@example.com");
        assert_eq!(result["emailVerified"], true);
        assert_eq!(result["groups"], serde_json::json!(["admins"]));
        assert_eq!(result["mappedRoles"], serde_json::json!(["Platform Admin"]));
        assert_eq!(result["overage"], false);

        let users_after: i64 = sqlx::query_scalar("SELECT count(*) FROM app_user")
            .fetch_one(&pool)
            .await?;
        let identities_after: i64 = sqlx::query_scalar("SELECT count(*) FROM auth_identity")
            .fetch_one(&pool)
            .await?;
        let sessions_after: i64 = sqlx::query_scalar("SELECT count(*) FROM session")
            .fetch_one(&pool)
            .await?;
        assert_eq!(users_before, users_after, "no app_user created");
        assert_eq!(
            identities_before, identities_after,
            "no auth_identity created"
        );
        assert_eq!(sessions_before, sessions_after, "no session created");
        Ok(())
    }

    /// `ADM-12` review round 1, B2: a draft whose provider differs from the
    /// saved one never sends anything to the saved provider — the token
    /// exchange goes to the draft's own discovery document.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_draft_whose_provider_differs_sends_nothing_to_the_saved_one(
        pool: sqlx::PgPool,
    ) -> sqlx::Result<()> {
        use lakehouse_test_support as _;

        // The saved connection's provider, reachable but never used here.
        let (saved_server, _saved_front, saved_issuer, _saved_http) = https_mock_provider().await;
        // The draft's own provider.
        let (draft_server, _draft_front, draft_issuer, draft_http) = https_mock_provider().await;
        let key = TestKey::generate("kid-1");
        mount_jwks(&draft_server, &key).await;

        lakehouse_store::sso_settings::upsert(
            &pool,
            &lakehouse_store::sso_settings::SsoSettings {
                oidc_issuer: Some(saved_issuer),
                oidc_client_id: Some("saved-client".to_owned()),
                oidc_client_secret: None,
                oidc_provider_name: Some("saved-provider".to_owned()),
                oidc_groups_claim: None,
                oidc_role_map: None,
                oidc_jit_provisioning: None,
                oidc_trust_email: None,
            },
            "test-user",
        )
        .await
        .unwrap();

        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), database_url_for(&pool));
        env.insert(
            "CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS".to_owned(),
            "true".to_owned(),
        );
        env.insert(
            "CONSOLE_PUBLIC_URL".to_owned(),
            "https://lakehouse.test".to_owned(),
        );
        env.insert("CONNECTOR_SECRETS_KEY".to_owned(), {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD.encode([0x2a_u8; 32])
        });
        let mut state = AppState::new(Config::from_map(&env).expect("a valid test Config"));
        // One client that trusts both mocks' CAs: the saved provider's CA
        // is only trusted, never dialled, by this test.
        state.sso_http_override = Some(draft_http);
        let admin = seeded_admin_id(&pool).await?;

        let (flow_cookie, _run_id) = start_sso_test(
            &state,
            admin,
            serde_json::json!({
                "issuer": draft_issuer,
                "clientId": "draft-client",
                "clientSecret": "draft-secret",
            }),
        )
        .await;
        let flow = decode_flow_cookie(&flow_cookie);
        let id_token = sign_id_token(
            &IdTokenClaims {
                sub: "draft-subject-2",
                iss: &draft_issuer,
                aud: "draft-client",
                exp: time::OffsetDateTime::now_utc().unix_timestamp() + 3600,
                nonce: &flow.nonce,
                email: None,
                email_verified: None,
                groups: None,
                claim_names: None,
            },
            &key,
        );
        mount_token_endpoint(&draft_server, &id_token).await;

        let response = finish_sso_test(&state, &flow_cookie, &flow, "code-2").await;
        assert_eq!(response.status(), StatusCode::FOUND);

        // The saved provider saw no request at all: not the discovery
        // document, and above all not its token endpoint.
        let saved_requests = saved_server.received_requests().await.unwrap();
        assert!(
            saved_requests.is_empty(),
            "the draft's test sign-in must not reach the saved provider: {saved_requests:?}"
        );
        // The draft's provider is where the code was exchanged.
        let draft_requests = draft_server.received_requests().await.unwrap();
        assert!(
            draft_requests
                .iter()
                .any(|request| request.method.as_str() == "POST" && request.url.path() == "/token"),
            "the exchange goes to the draft's token endpoint: {draft_requests:?}"
        );
        Ok(())
    }

    /// `ADM-12` review round 1, S1: a provider that cannot be reached is
    /// told apart from one that refused the client, with a fixed sentence.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_test_sign_in_that_cannot_reach_the_provider_says_so(
        pool: sqlx::PgPool,
    ) -> sqlx::Result<()> {
        use lakehouse_test_support as _;

        // A port nothing listens on: bound, then released.
        let closed_port = {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            listener.local_addr().unwrap().port()
        };
        let server = wiremock::MockServer::start().await;
        let plain_port: u16 = url::Url::parse(&server.uri()).unwrap().port().unwrap();
        let front = crate::tls_proxy::HttpsFront::start("127.0.0.1", plain_port, "127.0.0.1").await;
        let issuer = format!("https://127.0.0.1:{}", front.port());
        // The discovery document is readable; only the token endpoint is
        // unreachable.
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path(
                "/.well-known/openid-configuration",
            ))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "issuer": issuer,
                    "authorization_endpoint": format!("{issuer}/authorize"),
                    "token_endpoint": format!("https://127.0.0.1:{closed_port}/token"),
                    "jwks_uri": format!("{issuer}/.well-known/jwks.json"),
                })),
            )
            .mount(&server)
            .await;
        let http = reqwest::Client::builder()
            .add_root_certificate(
                reqwest::Certificate::from_pem(front.ca_pem().as_bytes()).unwrap(),
            )
            .build()
            .unwrap();

        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), database_url_for(&pool));
        env.insert(
            "CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS".to_owned(),
            "true".to_owned(),
        );
        env.insert(
            "CONSOLE_PUBLIC_URL".to_owned(),
            "https://lakehouse.test".to_owned(),
        );
        env.insert("CONNECTOR_SECRETS_KEY".to_owned(), {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD.encode([0x2a_u8; 32])
        });
        let mut state = AppState::new(Config::from_map(&env).expect("a valid test Config"));
        state.sso_http_override = Some(http);
        let admin = seeded_admin_id(&pool).await?;

        let (flow_cookie, run_id) = start_sso_test(
            &state,
            admin,
            serde_json::json!({
                "issuer": issuer,
                "clientId": "draft-client",
                "clientSecret": "draft-secret",
            }),
        )
        .await;
        let flow = decode_flow_cookie(&flow_cookie);
        let response = finish_sso_test(&state, &flow_cookie, &flow, "code-3").await;
        assert_eq!(response.status(), StatusCode::FOUND);

        let run = lakehouse_store::sso_settings::get_test_run(&pool, run_id)
            .await
            .unwrap()
            .expect("the run exists");
        let result = run.result.expect("the callback stored a result");
        assert_eq!(result["ok"], false, "{result}");
        assert_eq!(
            result["error"],
            "The identity provider could not be reached."
        );
        Ok(())
    }

    /// `ADM-12` review round 1, S4: a run with no secret of its own sends
    /// none — never the deployment's `OIDC_CLIENT_SECRET`.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_test_run_with_no_secret_sends_no_client_secret(
        pool: sqlx::PgPool,
    ) -> sqlx::Result<()> {
        use lakehouse_test_support as _;

        let (server, _front, issuer, http) = https_mock_provider().await;
        let key = TestKey::generate("kid-1");
        mount_jwks(&server, &key).await;

        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), database_url_for(&pool));
        env.insert(
            "CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS".to_owned(),
            "true".to_owned(),
        );
        env.insert(
            "CONSOLE_PUBLIC_URL".to_owned(),
            "https://lakehouse.test".to_owned(),
        );
        env.insert("CONNECTOR_SECRETS_KEY".to_owned(), {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD.encode([0x2a_u8; 32])
        });
        // The deployment's own secret, which this run must not use.
        env.insert("OIDC_CLIENT_SECRET".to_owned(), "env-secret".to_owned());
        let mut state = AppState::new(Config::from_map(&env).expect("a valid test Config"));
        state.sso_http_override = Some(http);
        let admin = seeded_admin_id(&pool).await?;

        let (flow_cookie, run_id) = start_sso_test(
            &state,
            admin,
            serde_json::json!({
                "issuer": issuer,
                "clientId": "draft-client",
                // An explicit null: this run carries no secret at all.
                "clientSecret": null,
            }),
        )
        .await;
        let flow = decode_flow_cookie(&flow_cookie);
        let id_token = sign_id_token(
            &IdTokenClaims {
                sub: "draft-subject-4",
                iss: &issuer,
                aud: "draft-client",
                exp: time::OffsetDateTime::now_utc().unix_timestamp() + 3600,
                nonce: &flow.nonce,
                email: None,
                email_verified: None,
                groups: None,
                claim_names: None,
            },
            &key,
        );
        mount_token_endpoint(&server, &id_token).await;

        let response = finish_sso_test(&state, &flow_cookie, &flow, "code-4").await;
        assert_eq!(response.status(), StatusCode::FOUND);

        let token_requests: Vec<_> = server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|request| request.method.as_str() == "POST" && request.url.path() == "/token")
            .collect();
        assert_eq!(token_requests.len(), 1, "the code was exchanged once");
        let body = String::from_utf8_lossy(&token_requests[0].body);
        assert!(
            !body.contains("client_secret"),
            "a run with no secret sends none, not OIDC_CLIENT_SECRET: {body}"
        );
        let run = lakehouse_store::sso_settings::get_test_run(&pool, run_id)
            .await
            .unwrap()
            .expect("the run exists");
        assert_eq!(
            run.result.expect("the callback stored a result")["ok"],
            true,
            "the public-client exchange still verifies the token"
        );
        Ok(())
    }

    /// `ADM-12` review round 1, S2: a token carrying an overage marker
    /// reports `overage: true` and maps no group-based role (`SEC-39`).
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_test_sign_in_with_a_group_overage_says_so(pool: sqlx::PgPool) -> sqlx::Result<()> {
        use lakehouse_test_support as _;

        let (server, _front, issuer, http) = https_mock_provider().await;
        let key = TestKey::generate("kid-1");
        mount_jwks(&server, &key).await;

        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), database_url_for(&pool));
        env.insert(
            "CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS".to_owned(),
            "true".to_owned(),
        );
        env.insert(
            "CONSOLE_PUBLIC_URL".to_owned(),
            "https://lakehouse.test".to_owned(),
        );
        let mut state = AppState::new(Config::from_map(&env).expect("a valid test Config"));
        state.sso_http_override = Some(http);
        let admin = seeded_admin_id(&pool).await?;

        let (flow_cookie, run_id) = start_sso_test(
            &state,
            admin,
            serde_json::json!({
                "issuer": issuer,
                "clientId": "draft-client",
                "roleMap": [{"group": "lakehouse-admins", "role": "Platform Admin"}],
            }),
        )
        .await;
        let flow = decode_flow_cookie(&flow_cookie);
        let id_token = sign_id_token(
            &IdTokenClaims {
                sub: "draft-subject-5",
                iss: &issuer,
                aud: "draft-client",
                exp: time::OffsetDateTime::now_utc().unix_timestamp() + 3600,
                nonce: &flow.nonce,
                email: None,
                email_verified: None,
                groups: None,
                claim_names: Some(serde_json::json!({ "groups": "src1" })),
            },
            &key,
        );
        mount_token_endpoint(&server, &id_token).await;

        let response = finish_sso_test(&state, &flow_cookie, &flow, "code-5").await;
        assert_eq!(response.status(), StatusCode::FOUND);
        let run = lakehouse_store::sso_settings::get_test_run(&pool, run_id)
            .await
            .unwrap()
            .expect("the run exists");
        let result = run.result.expect("the callback stored a result");
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["overage"], true, "{result}");
        assert_eq!(result["mappedRoles"], serde_json::json!([]), "{result}");
        Ok(())
    }

    /// The seeded Platform Admin as the route handlers see it: unrestricted
    /// (`*:*`), which is what `PUT`/`POST`/`GET` of SSO settings require (D5).
    fn admin_principal(id: Uuid) -> Principal {
        Principal {
            id: PrincipalId::User(id),
            tenant_ids: Vec::new(),
            display_name: "fajar".to_owned(),
            permissions: lakehouse_auth::PermissionSet::parse("*:*"),
            provider: "local".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    async fn seeded_admin_id(pool: &sqlx::PgPool) -> sqlx::Result<Uuid> {
        let (admin_id,): (Uuid,) = sqlx::query_as("SELECT id FROM app_user WHERE email = $1")
            .bind("fajar@meridian.example")
            .fetch_one(pool)
            .await?;
        Ok(admin_id)
    }

    /// Start a test sign-in by calling the route handler (`ADM-12` D8): the
    /// response's flow cookie and the run's id.
    async fn start_sso_test(
        state: &AppState,
        admin: Uuid,
        draft: serde_json::Value,
    ) -> (String, Uuid) {
        let response = crate::routes::settings::post_sso_test(
            axum::extract::State(state.clone()),
            axum::Extension(admin_principal(admin)),
            axum::body::Bytes::from(draft.to_string().into_bytes()),
        )
        .await
        .expect("a usable draft starts a test sign-in");
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "the draft must be usable: {:?}",
            response.status()
        );
        let flow_cookie = response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .find_map(|value| {
                let text = value.to_str().ok()?;
                text.starts_with("lh_oidc_flow=").then(|| text.to_owned())
            })
            .expect("the test flow cookie");
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let run_id: Uuid = body["id"]
            .as_str()
            .expect("the run id")
            .parse()
            .expect("a UUID");
        (flow_cookie, run_id)
    }

    /// Finish a test sign-in by calling the callback handler with the flow
    /// cookie `start_sso_test` returned.
    async fn finish_sso_test(
        state: &AppState,
        flow_cookie: &str,
        flow: &OidcFlow,
        code: &str,
    ) -> Response {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(flow_cookie.split(';').next().unwrap_or_default())
                .expect("an ASCII cookie"),
        );
        oidc_callback(
            axum::extract::State(state.clone()),
            axum::extract::Query(OidcCallbackQuery {
                code: Some(code.to_owned()),
                state: Some(flow.state.clone()),
            }),
            headers,
        )
        .await
        .expect("the callback answers")
    }

    fn sign_id_token(claims: &IdTokenClaims<'_>, key: &TestKey) -> String {
        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
        header.kid = Some(key.kid.clone());
        jsonwebtoken::encode(&header, claims, &key.encoding_key).expect("sign")
    }

    async fn mount_jwks(server: &wiremock::MockServer, key: &TestKey) {
        let set = jsonwebtoken::jwk::JwkSet {
            keys: vec![key.jwk.clone()],
        };
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/.well-known/jwks.json"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(&set))
            .mount(server)
            .await;
    }

    /// Mount the discovery document `SEC-38` reads when `OIDC_JWKS_URL` is
    /// unset. These round-trip tests set `OIDC_ISSUER` to the mock server
    /// itself and no `OIDC_JWKS_URL`, so this (not the key set's path)
    /// is what names `/.well-known/jwks.json` as the key set's address.
    async fn mount_discovery(server: &wiremock::MockServer, issuer: &str) {
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path(
                "/.well-known/openid-configuration",
            ))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "issuer": issuer,
                    "jwks_uri": format!("{issuer}/.well-known/jwks.json"),
                })),
            )
            .mount(server)
            .await;
    }

    async fn mount_token_endpoint(server: &wiremock::MockServer, id_token: &str) {
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/token"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "id_token": id_token })),
            )
            .mount(server)
            .await;
    }

    /// Same idiom `routes::pipelines::tests::database_url_for` uses: read
    /// the real connection info `#[sqlx::test]` set up for `pool` back out
    /// as a `DATABASE_URL` string, since `Config`/`AppState::new` only
    /// know how to take a URL, not an already-open pool.
    fn database_url_for(pool: &sqlx::PgPool) -> String {
        let options = pool.connect_options();
        format!(
            "postgres://{}:postgres@{}:{}/{}",
            options.get_username(),
            options.get_host(),
            options.get_port(),
            options
                .get_database()
                .expect("#[sqlx::test] always targets a named database")
        )
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn oidc_callback_with_a_matching_state_and_nonce_creates_a_session_and_redirects(
        pool: sqlx::PgPool,
    ) -> sqlx::Result<()> {
        use lakehouse_test_support as _;

        let server = wiremock::MockServer::start().await;
        let key = TestKey::generate("kid-1");
        mount_jwks(&server, &key).await;
        mount_discovery(&server, &server.uri()).await;

        let issuer = server.uri();
        let client_id = "lakehouse-console";
        let flow = OidcFlow {
            state: "state-from-start".to_owned(),
            nonce: "nonce-from-start".to_owned(),
            code_verifier: "verifier-1".to_owned(),
            next: "/dashboards".to_owned(),
            test_run_id: None,
        };

        let id_token = sign_id_token(
            &IdTokenClaims {
                sub: "user-a5-round-trip",
                iss: &issuer,
                aud: client_id,
                // 32-bit-safe: this repository's test process clock is
                // always well under `i32::MAX` seconds past the epoch.
                exp: time::OffsetDateTime::now_utc().unix_timestamp() + 3600,
                nonce: &flow.nonce,
                email: None,
                email_verified: None,
                groups: None,
                claim_names: None,
            },
            &key,
        );
        mount_token_endpoint(&server, &id_token).await;

        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), database_url_for(&pool));
        env.insert("OIDC_ISSUER".to_owned(), issuer.clone());
        env.insert("OIDC_CLIENT_ID".to_owned(), client_id.to_owned());
        env.insert(
            "OIDC_AUTHORIZE_URL".to_owned(),
            format!("{issuer}/authorize"),
        );
        env.insert("OIDC_TOKEN_URL".to_owned(), format!("{issuer}/token"));
        env.insert(
            "OIDC_REDIRECT_URI".to_owned(),
            "https://lake.invalid/api/auth/oidc/callback".to_owned(),
        );
        // Off by default (see `oidc_callback`'s doc comment: JIT
        // provisioning stays off unless an operator opts in) — turned on here
        // because this test's whole point is a *successful* round trip for
        // a `sub` this deployment has never seen before, which is exactly
        // what JIT provisioning exists for.
        env.insert("OIDC_JIT_PROVISIONING".to_owned(), "true".to_owned());
        let config = Config::from_map(&env).expect("a valid test Config");
        let state = AppState::new(config);
        let app = crate::routes::router(state);

        let cookie = oidc_flow_cookie_header(true, Some(&flow));
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/auth/oidc/callback?code=auth-code-1&state={}",
                        flow.state
                    ))
                    .header(header::COOKIE, cookie.to_str().unwrap())
                    .header(header::USER_AGENT, "QA-Browser/1.0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FOUND);
        assert_eq!(
            response.headers().get(header::LOCATION).unwrap(),
            "/dashboards"
        );
        let set_cookies: Vec<&str> = response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap())
            .collect();
        // Both the new session cookie AND the flow cookie's clear must be
        // present — the flow cookie is single-use even on success (see
        // `oidc_callback`'s doc comment).
        assert!(
            set_cookies
                .iter()
                .any(|c| c.starts_with(&format!("{SESSION_COOKIE_NAME}="))
                    && !c.contains("Max-Age=0")),
            "expected a fresh, non-empty session cookie: {set_cookies:?}"
        );
        assert!(
            set_cookies
                .iter()
                .any(|c| c.starts_with(&format!("{OIDC_FLOW_COOKIE_NAME}="))
                    && c.contains("Max-Age=0")),
            "expected the flow cookie to be cleared even on success: {set_cookies:?}"
        );

        // QA LH-SEC-5-01 (ADM-14 D9/D10): the single sign-on callback records
        // the browser it came from, and records no network address.
        let (count, user_agent, created_ip): (i64, Option<String>, Option<String>) =
            sqlx::query_as("SELECT count(*), max(user_agent), max(created_ip) FROM session")
                .fetch_one(&pool)
                .await?;
        assert_eq!(count, 1, "one callback, one session");
        assert_eq!(user_agent.as_deref(), Some("QA-Browser/1.0"));
        assert_eq!(
            created_ip, None,
            "the network address is not recorded (ADM-14 D10)"
        );
        Ok(())
    }

    // ── `SEC-39`: a group overage is said, not silent ──────────────────────

    /// The overage marker rides on the landing path as a fixed code, and a
    /// path that already carries a query keeps it.
    #[test]
    fn the_sign_in_notice_is_appended_without_dropping_an_existing_query() {
        assert_eq!(
            path_with_notice("/", "group_overage"),
            "/?ssoNotice=group_overage"
        );
        assert_eq!(
            path_with_notice("/dashboards/main?tab=history", "group_overage"),
            "/dashboards/main?tab=history&ssoNotice=group_overage"
        );
    }

    /// A successful round trip whose id token carries `_claim_names`
    /// instead of `groups` (Entra ID's overage marker) still signs in, and
    /// the redirect tells the console so (`SEC-39`, decisions 2 and 4).
    /// Same wiremock harness as
    /// `oidc_callback_with_a_matching_state_and_nonce_creates_a_session_and_redirects`;
    /// the only differences are the marker and the asserted `LOCATION`.
    #[sqlx::test(migrations = "../../migrations")]
    async fn oidc_callback_with_a_group_overage_redirects_with_the_notice(
        pool: sqlx::PgPool,
    ) -> sqlx::Result<()> {
        use lakehouse_test_support as _;

        let server = wiremock::MockServer::start().await;
        let key = TestKey::generate("kid-1");
        mount_jwks(&server, &key).await;
        mount_discovery(&server, &server.uri()).await;

        let issuer = server.uri();
        let client_id = "lakehouse-console";
        let flow = OidcFlow {
            state: "state-from-start".to_owned(),
            nonce: "nonce-from-start".to_owned(),
            code_verifier: "verifier-1".to_owned(),
            next: "/dashboards".to_owned(),
            test_run_id: None,
        };

        let id_token = sign_id_token(
            &IdTokenClaims {
                sub: "user-overage-round-trip",
                iss: &issuer,
                aud: client_id,
                exp: time::OffsetDateTime::now_utc().unix_timestamp() + 3600,
                nonce: &flow.nonce,
                email: None,
                email_verified: None,
                groups: None,
                claim_names: Some(serde_json::json!({ "groups": "src1" })),
            },
            &key,
        );
        mount_token_endpoint(&server, &id_token).await;

        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), database_url_for(&pool));
        env.insert("OIDC_ISSUER".to_owned(), issuer.clone());
        env.insert("OIDC_CLIENT_ID".to_owned(), client_id.to_owned());
        env.insert(
            "OIDC_AUTHORIZE_URL".to_owned(),
            format!("{issuer}/authorize"),
        );
        env.insert("OIDC_TOKEN_URL".to_owned(), format!("{issuer}/token"));
        env.insert(
            "OIDC_REDIRECT_URI".to_owned(),
            "https://lake.invalid/api/auth/oidc/callback".to_owned(),
        );
        env.insert("OIDC_JIT_PROVISIONING".to_owned(), "true".to_owned());
        // A mapping that COULD have applied had the token carried the
        // group: the point is that an overage maps nothing and says so,
        // not that this deployment has no mapping at all.
        env.insert(
            "OIDC_ROLE_MAP".to_owned(),
            "lakehouse-admins=Platform Admin".to_owned(),
        );
        let config = Config::from_map(&env).expect("a valid test Config");
        let state = AppState::new(config);
        let app = crate::routes::router(state);

        let cookie = oidc_flow_cookie_header(true, Some(&flow));
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/auth/oidc/callback?code=auth-code-1&state={}",
                        flow.state
                    ))
                    .header(header::COOKIE, cookie.to_str().unwrap())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FOUND);
        assert_eq!(
            response.headers().get(header::LOCATION).unwrap(),
            "/dashboards?ssoNotice=group_overage"
        );
        Ok(())
    }

    /// `SEC-65` D2/AC2: the provider did not confirm the token's email and
    /// an existing account holds it, so the callback sends the person back
    /// to the sign-in page with the fixed notice code — not the generic 401
    /// body — and mints nothing: no session cookie, no identity row. The
    /// audit row holds the provider and the subject, never the email.
    #[sqlx::test(migrations = "../../migrations")]
    async fn oidc_callback_redirects_an_unconfirmed_email_to_the_sign_in_notice(
        pool: sqlx::PgPool,
    ) -> sqlx::Result<()> {
        use lakehouse_test_support as _;

        let server = wiremock::MockServer::start().await;
        let key = TestKey::generate("kid-1");
        mount_jwks(&server, &key).await;
        mount_discovery(&server, &server.uri()).await;

        let issuer = server.uri();
        let client_id = "lakehouse-console";
        let flow = OidcFlow {
            state: "state-from-start".to_owned(),
            nonce: "nonce-from-start".to_owned(),
            code_verifier: "verifier-1".to_owned(),
            next: "/dashboards".to_owned(),
            test_run_id: None,
        };

        // Rina Wijaya (seeded, `0002_seed_identity.sql`) is an active
        // account under this address; the token claims it but does not
        // confirm it (`email_verified` absent).
        let id_token = sign_id_token(
            &IdTokenClaims {
                sub: "unconfirmed-callback-subject",
                iss: &issuer,
                aud: client_id,
                exp: time::OffsetDateTime::now_utc().unix_timestamp() + 3600,
                nonce: &flow.nonce,
                email: Some("rina@meridian.example"),
                email_verified: None,
                groups: None,
                claim_names: None,
            },
            &key,
        );
        mount_token_endpoint(&server, &id_token).await;

        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), database_url_for(&pool));
        env.insert("OIDC_ISSUER".to_owned(), issuer.clone());
        env.insert("OIDC_CLIENT_ID".to_owned(), client_id.to_owned());
        env.insert(
            "OIDC_AUTHORIZE_URL".to_owned(),
            format!("{issuer}/authorize"),
        );
        env.insert("OIDC_TOKEN_URL".to_owned(), format!("{issuer}/token"));
        env.insert(
            "OIDC_REDIRECT_URI".to_owned(),
            "https://lake.invalid/api/auth/oidc/callback".to_owned(),
        );
        env.insert("OIDC_JIT_PROVISIONING".to_owned(), "true".to_owned());
        let config = Config::from_map(&env).expect("a valid test Config");
        let state = AppState::new(config);
        let app = crate::routes::router(state);

        let cookie = oidc_flow_cookie_header(true, Some(&flow));
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/auth/oidc/callback?code=auth-code-1&state={}",
                        flow.state
                    ))
                    .header(header::COOKIE, cookie.to_str().unwrap())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FOUND);
        assert_eq!(
            response.headers().get(header::LOCATION).unwrap(),
            "/login?ssoNotice=email_not_verified",
            "the refusal goes to the sign-in page's notice, with a fixed code"
        );
        let set_cookies: Vec<&str> = response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap())
            .collect();
        assert!(
            !set_cookies
                .iter()
                .any(|c| c.starts_with(&format!("{SESSION_COOKIE_NAME}="))),
            "a refused sign-in mints no session: {set_cookies:?}"
        );
        assert!(
            set_cookies
                .iter()
                .any(|c| c.starts_with(&format!("{OIDC_FLOW_COOKIE_NAME}="))
                    && c.contains("Max-Age=0")),
            "the flow cookie is still single-use: {set_cookies:?}"
        );

        let identities: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM auth_identity WHERE provider = 'oidc:test' \
             AND external_subject = 'unconfirmed-callback-subject'",
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(identities, 0, "nothing was linked");

        let audited: (String,) = sqlx::query_as(
            "SELECT args->>'subject' FROM audit_event \
             WHERE action = 'auth.oidc.link_refused' AND outcome = 'refused'",
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(audited.0, "unconfirmed-callback-subject");
        let with_email: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM audit_event WHERE args::text LIKE '%rina@meridian.example%'",
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(with_email, 0, "the audit row never carries the email");
        Ok(())
    }

    // ── `oidc_callback` writes a real `audit_event` row on
    // success. Reuses the wiremock round-trip harness above verbatim —
    // the only addition is the post-response SELECT against
    // `audit_event`. ───────────────────────────────────────────────────────

    /// A successful OIDC round trip writes a `audit_event` row with
    /// `action = 'auth.oidc.login'`, `principal_kind = 'user'`, and
    /// `outcome = 'executed'`. `resource_id` is intentionally `None`
    /// (no second `SELECT` for the session row's id — see `oidc_callback`'s
    /// own audit-write comment).
    #[sqlx::test(migrations = "../../migrations")]
    async fn oidc_callback_writes_an_audit_event_with_principal_kind_user_on_success(
        pool: sqlx::PgPool,
    ) -> sqlx::Result<()> {
        use lakehouse_test_support as _;

        // Same wiremock round-trip shape as
        // `oidc_callback_with_a_matching_state_and_nonce_creates_a_session_and_redirects`:
        // mock JWKS + token endpoint, sign an id token with the matching
        // nonce, drive the round trip end to end. The only difference is
        // the post-response SELECT against `audit_event`.
        let server = wiremock::MockServer::start().await;
        let key = TestKey::generate("kid-1");
        mount_jwks(&server, &key).await;
        mount_discovery(&server, &server.uri()).await;

        let issuer = server.uri();
        let client_id = "lakehouse-console";
        let flow = OidcFlow {
            state: "state-from-start".to_owned(),
            nonce: "nonce-from-start".to_owned(),
            code_verifier: "verifier-1".to_owned(),
            next: "/dashboards".to_owned(),
            test_run_id: None,
        };

        let id_token = sign_id_token(
            &IdTokenClaims {
                sub: "user-g1-audit-round-trip",
                iss: &issuer,
                aud: client_id,
                exp: time::OffsetDateTime::now_utc().unix_timestamp() + 3600,
                nonce: &flow.nonce,
                email: None,
                email_verified: None,
                groups: None,
                claim_names: None,
            },
            &key,
        );
        mount_token_endpoint(&server, &id_token).await;

        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), database_url_for(&pool));
        env.insert("OIDC_ISSUER".to_owned(), issuer.clone());
        env.insert("OIDC_CLIENT_ID".to_owned(), client_id.to_owned());
        env.insert(
            "OIDC_AUTHORIZE_URL".to_owned(),
            format!("{issuer}/authorize"),
        );
        env.insert("OIDC_TOKEN_URL".to_owned(), format!("{issuer}/token"));
        env.insert(
            "OIDC_REDIRECT_URI".to_owned(),
            "https://lake.invalid/api/auth/oidc/callback".to_owned(),
        );
        env.insert("OIDC_JIT_PROVISIONING".to_owned(), "true".to_owned());
        let config = Config::from_map(&env).expect("a valid test Config");
        let state = AppState::new(config);
        let app = crate::routes::router(state);

        let cookie = oidc_flow_cookie_header(true, Some(&flow));
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/auth/oidc/callback?code=auth-code-1&state={}",
                        flow.state
                    ))
                    .header(header::COOKIE, cookie.to_str().unwrap())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FOUND);

        let row: (String, Option<String>, String) = sqlx::query_as(
            "SELECT action, principal_kind, outcome FROM audit_event \
             WHERE action = 'auth.oidc.login' ORDER BY at DESC LIMIT 1",
        )
        .fetch_one(&pool)
        .await
        .expect("an audit_event row for auth.oidc.login must exist");

        assert_eq!(row.0, "auth.oidc.login");
        assert_eq!(
            row.1.as_deref(),
            Some("user"),
            "principal_kind must reflect the resolved PrincipalId::User — \
             see Principal::kind_for_audit's mapping"
        );
        assert_eq!(row.2, "executed");
        Ok(())
    }

    // ── `GET /api/auth/providers` tests, written test-first
    // (before `providers` exists at all). ────────────────────────────────────

    /// `GET path` against a fresh router built from `state`, decoded as a
    /// JSON body. This is a
    /// local equivalent built from the exact `to_bytes` +
    /// `serde_json::from_slice::<Value>` idiom
    /// `routes::knowledge::tests::every_database_backed_route_returns_503_without_a_pool`
    /// already uses, rather than inventing a second body-reading pattern.
    async fn get_json(state: &AppState, path: &str) -> serde_json::Value {
        let app = crate::routes::router(state.clone());
        let response = app
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// `AppState` with no Postgres pool — same shape every other `tests`
    /// module in this crate uses for the "no pool → 503" branch
    /// (`routes::identity::tests::state_without_pool`,
    /// `routes::knowledge::tests`, ...). The D2-route tests that don't
    /// reach [`pool`] (the 400 non-UUID path, the 404 service-principal
    /// path) use this so they don't depend on a testcontainer Postgres.
    fn state_without_pool() -> AppState {
        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), "not a postgres url".to_owned());
        AppState::new(Config::from_map(&env).unwrap())
    }

    /// Read the bytes out of an axum `Response` body. Used by the
    /// 404-body byte-equality test, which compares two responses'
    /// bodies exactly.
    async fn body_bytes(response: Response) -> Vec<u8> {
        axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec()
    }

    /// The same bytes, decoded as JSON — the shape a route test that must
    /// read a body (`GET /api/settings/sso`, `/api/auth/providers`) uses.
    async fn body_json(response: Response) -> serde_json::Value {
        serde_json::from_slice(&body_bytes(response).await).unwrap()
    }

    #[tokio::test]
    async fn providers_reports_oidc_configured_true_only_when_auth_state_has_it() {
        let with_oidc = state_with_oidc_flow_config();
        let body = get_json(&with_oidc, "/api/auth/providers").await;
        assert_eq!(body["oidc"], serde_json::json!(true));

        let without_oidc = AppState::new(Config::from_map(&HashMap::new()).unwrap());
        let body = get_json(&without_oidc, "/api/auth/providers").await;
        assert_eq!(body["oidc"], serde_json::json!(false));
    }

    /// `ADM-12` review round 2, R2-B1: with only `OIDC_ISSUER` and
    /// `OIDC_CLIENT_ID` set — the shape a deployment that accepts bearer
    /// tokens uses — the authenticator exists (the bearer path's fact), but
    /// no sign-in can be offered, so `providers` reports no OIDC and the
    /// sign-in page shows no button that would fail.
    #[tokio::test]
    async fn an_issuer_and_a_client_id_alone_report_no_sign_in_but_keep_the_authenticator() {
        let mut env = HashMap::new();
        env.insert(
            "DATABASE_URL".to_owned(),
            "postgres://user:pass@127.0.0.1:1/nonexistent_db_xyz".to_owned(),
        );
        env.insert(
            "OIDC_ISSUER".to_owned(),
            "https://idp.example.com".to_owned(),
        );
        env.insert("OIDC_CLIENT_ID".to_owned(), "lakehouse-console".to_owned());
        let state = AppState::new(Config::from_map(&env).expect("a valid test Config"));

        // The bearer-token fact: an authenticator exists.
        assert!(
            crate::runtime_settings::oidc(&state).await.is_some(),
            "an issuer and a client ID alone must still verify bearer tokens"
        );

        // The sign-in fact: no authorize, token or redirect address, so no
        // button.
        let body = get_json(&state, "/api/auth/providers").await;
        assert_eq!(body["oidc"], serde_json::json!(false));
        assert_eq!(body["providerName"], serde_json::Value::Null);
    }

    /// `ADM-12` review round 2, R2-B1, the end-to-end half: a deployment
    /// that sets only `OIDC_ISSUER` and `OIDC_CLIENT_ID` (no sign-in
    /// address) still accepts a bearer token, exactly as on `main`.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_bearer_token_is_accepted_with_only_an_issuer_and_a_client_id(
        pool: sqlx::PgPool,
    ) -> sqlx::Result<()> {
        use lakehouse_test_support as _;

        let server = wiremock::MockServer::start().await;
        let key = TestKey::generate("kid-1");
        mount_jwks(&server, &key).await;
        mount_discovery(&server, &server.uri()).await;
        let issuer = server.uri();

        // A user with an `OIDC` identity already linked, so the bearer path
        // needs no JIT provisioning. `OIDC_PROVIDER_NAME` is unset, so the
        // label is `oidc:default`.
        let user_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO app_user (id, name, username, email, status) \
             VALUES ($1, 'Bearer Only', 'bearer-only', 'bearer-only@meridian.example', 'active')",
        )
        .bind(user_id)
        .execute(&pool)
        .await?;
        sqlx::query(
            "INSERT INTO auth_identity (provider, external_subject, app_user_id, password_hash) \
             VALUES ('oidc:default', 'bearer-only-subject', $1, NULL)",
        )
        .bind(user_id)
        .execute(&pool)
        .await?;

        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), database_url_for(&pool));
        env.insert("OIDC_ISSUER".to_owned(), issuer.clone());
        env.insert("OIDC_CLIENT_ID".to_owned(), "lakehouse-console".to_owned());
        let state = AppState::new(Config::from_map(&env).expect("a valid test Config"));
        let app = crate::routes::router(state);

        let token = sign_id_token(
            &IdTokenClaims {
                sub: "bearer-only-subject",
                iss: &issuer,
                aud: "lakehouse-console",
                exp: (time::OffsetDateTime::now_utc() + time::Duration::minutes(5))
                    .unix_timestamp(),
                nonce: "not-checked-on-the-bearer-path",
                email: None,
                email_verified: None,
                groups: None,
                claim_names: None,
            },
            &key,
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/auth/me")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "a bearer token must still be accepted when only the issuer and the client ID are set"
        );
        Ok(())
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn saving_a_config_makes_the_next_providers_call_report_oidc_with_no_restart(
        pool: sqlx::PgPool,
    ) -> sqlx::Result<()> {
        // `ADM-12` D10: a saved connection is ready only when it can really
        // be used, so this needs a provider whose discovery document answers
        // — the same `https` mock the sign-in test uses.
        let (server, _front, issuer, http) = https_mock_provider().await;
        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), database_url_for(&pool));
        env.insert(
            "CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS".to_owned(),
            "true".to_owned(),
        );
        env.insert(
            "CONSOLE_PUBLIC_URL".to_owned(),
            "https://lakehouse.test".to_owned(),
        );
        let mut state = AppState::new(Config::from_map(&env).expect("valid config"));
        state.sso_http_override = Some(http);

        // At first, no OIDC is configured
        let body = get_json(&state, "/api/auth/providers").await;
        assert_eq!(body["oidc"], serde_json::json!(false));

        // Save SSO configuration to the database
        lakehouse_store::sso_settings::upsert(
            &pool,
            &lakehouse_store::sso_settings::SsoSettings {
                oidc_issuer: Some(issuer),
                oidc_client_id: Some("saved-client-id".to_owned()),
                oidc_client_secret: None,
                oidc_provider_name: Some("saved-provider".to_owned()),
                oidc_groups_claim: None,
                oidc_role_map: None,
                oidc_jit_provisioning: None,
                oidc_trust_email: None,
            },
            "test-user",
        )
        .await
        .unwrap();

        // Clear cache as a console save does
        state.sso_settings_cache.clear().await;
        *state.oidc.write().await = None;

        // Next call reports oidc: true and provider_name without restart!
        let body = get_json(&state, "/api/auth/providers").await;
        assert_eq!(body["oidc"], serde_json::json!(true));
        assert_eq!(body["providerName"], serde_json::json!("saved-provider"));
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .any(|request| request.url.path() == "/.well-known/openid-configuration"),
            "the addresses come from the saved issuer's discovery document"
        );

        Ok(())
    }

    #[tokio::test]
    async fn an_http_or_internal_issuer_gives_400() {
        use crate::internal_hosts::InternalHosts;
        use crate::routes::settings::validate_sso_issuer;

        let internal_hosts = InternalHosts::NONE;

        // http:// is refused
        let err = validate_sso_issuer("http://idp.example.com", &internal_hosts)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), crate::routes::settings::ISSUER_NOT_HTTPS);

        // internal address (e.g. 127.0.0.1) is refused
        let err = validate_sso_issuer("https://127.0.0.1:8443", &internal_hosts)
            .await
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            crate::routes::settings::ISSUER_INTERNAL_REFUSED
        );
    }

    #[test]
    fn an_unknown_role_in_the_map_gives_400() {
        use crate::routes::settings::validate_role_map_entries;

        let known_roles = vec!["Platform Admin".to_owned(), "Analyst".to_owned()];

        let bad_map = serde_json::json!([
            {"group": "admins", "role": "Platform Admin"},
            {"group": "devs", "role": "UnknownRole"}
        ]);
        let items = bad_map.as_array().unwrap();
        let err = validate_role_map_entries(items, &known_roles).unwrap_err();
        assert_eq!(err.to_string(), "Unknown role: \"UnknownRole\".");

        let good_map = serde_json::json!([
            {"group": "admins", "role": "Platform Admin"},
            {"group": "devs", "role": "Analyst"}
        ]);
        let items = good_map.as_array().unwrap();
        assert!(validate_role_map_entries(items, &known_roles).is_ok());
    }

    // ── `GET /api/auth/sessions` routing shape ──────

    /// A service principal — even one that nominally holds
    /// `identity:sessions:manage` — must yield [`SessionsDecision::Empty`]
    /// from [`sessions_decision_for`]. A service identity has no
    /// `app_user_id` of its own to filter by, and the admin path is
    /// "every `app_user_id`", so flipping `is_admin = true` for a service
    /// principal would hand it every session in the deployment — exactly
    /// the enumeration a service caller has no business performing. This
    /// pins down that the service-principal short-circuit happens BEFORE
    /// the permission check, not after.
    #[test]
    fn sessions_decision_is_empty_for_every_service_principal() {
        use lakehouse_auth::PermissionSet;

        let admin_service = Principal {
            id: PrincipalId::Service(Uuid::new_v4()),
            tenant_ids: Vec::new(),
            display_name: "service-with-admin".to_owned(),
            permissions: PermissionSet::parse("identity:sessions:manage"),
            provider: "service".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        };
        assert_eq!(
            sessions_decision_for(&admin_service),
            SessionsDecision::Empty,
            "a service principal must not enumerate, even with admin permission"
        );

        let plain_service = Principal {
            id: PrincipalId::Service(Uuid::new_v4()),
            tenant_ids: Vec::new(),
            display_name: "service-without-admin".to_owned(),
            permissions: PermissionSet::default(),
            provider: "service".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        };
        assert_eq!(
            sessions_decision_for(&plain_service),
            SessionsDecision::Empty,
            "a service principal without the admin permission must also yield empty"
        );
    }

    /// The two human branches of [`sessions_decision_for`]: a user without
    /// `identity:sessions:manage` sees only their own sessions, and a user
    /// with it sees every session. The `Uuid` is preserved across the
    /// match so the SQL can keep using a stable, caller-bound id rather
    /// than re-deriving one.
    #[test]
    fn sessions_decision_for_a_user_principal_flips_on_identity_sessions_manage() {
        use lakehouse_auth::PermissionSet;

        let user_id = Uuid::new_v4();
        let plain_user = Principal {
            id: PrincipalId::User(user_id),
            tenant_ids: Vec::new(),
            display_name: "plain-user".to_owned(),
            permissions: PermissionSet::default(),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        };
        assert_eq!(
            sessions_decision_for(&plain_user),
            SessionsDecision::Own(user_id),
        );

        let admin_user = Principal {
            permissions: PermissionSet::parse("identity:sessions:manage"),
            ..plain_user
        };
        assert_eq!(
            sessions_decision_for(&admin_user),
            SessionsDecision::All(user_id),
        );
    }

    // ── `DELETE /api/auth/sessions/{id}` routing shape ──

    /// A non-UUID path segment is a 400, not a 404. A malformed id can't
    /// match any row, so 404'ing it would render the same "doesn't
    /// exist" body as a foreign / missing id — which a caller could
    /// otherwise use to tell "your path is malformed" apart from "the
    /// session id is foreign/nonexistent." The 400 makes the
    /// malformed-path case its own shape, so the route's 404 stays a
    /// pure "is this id yours / does it exist" signal.
    ///
    /// Reachable without a pool: the UUID parse happens before
    /// [`pool`], so `state_without_pool` exercises this branch in
    /// isolation (and would 503 on `pool` if the path were valid).
    #[tokio::test]
    async fn delete_session_route_rejects_a_non_uuid_path_segment_with_400() {
        let state = state_without_pool();
        let principal = AuthenticatedPrincipal(Principal {
            id: PrincipalId::User(Uuid::new_v4()),
            tenant_ids: Vec::new(),
            display_name: "alice".to_owned(),
            permissions: lakehouse_auth::PermissionSet::default(),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        });
        let rejection = revoke_session(State(state), principal, Path("not-a-uuid".to_owned()))
            .await
            .expect_err("a non-UUID path segment must be a rejection");
        let response = rejection.into_response();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "a non-UUID path must be 400, not 404 — it must not render the same body as a foreign/missing id"
        );
    }

    /// The two 404 arms (foreign id, missing id) must
    /// produce byte-equal responses, so the route never reveals whether
    /// another user's session id exists. A future reader who changes the
    /// foreign-id message has to consciously change the missing-id
    /// message too — the route is not an enumeration oracle.
    ///
    /// The test does NOT hit the route end-to-end (that needs a real
    /// DB); it constructs both response paths directly. The two paths
    /// are:
    ///
    /// * Foreign / missing id: `revoke_session_as_caller` returns
    ///   `SessionOwnershipError::NotFound`, which `?`-converts through
    ///   `From<SessionOwnershipError> for StoreError` to
    ///   `StoreError::NotFound`, then `From<StoreError> for ApiError`
    ///   to `ApiError::NotFound("record not found")`.
    /// * Service principal: the handler returns
    ///   `ApiError::NotFound("record not found".to_owned())` directly,
    ///   bypassing the DB.
    ///
    /// Both must render the same `{"error": "record not found"}` body.
    #[tokio::test]
    async fn delete_session_route_404_body_for_foreign_id_matches_missing_id() {
        use crate::error::ApiRejection;

        // Foreign / missing id path: SessionOwnershipError::NotFound ->
        // StoreError::NotFound -> ApiError::NotFound("record not found").
        let via_store: Response = ApiRejection(StoreError::NotFound.into()).into_response();

        // Service-principal path: the handler short-circuits with the
        // same ApiError::NotFound body before touching the DB.
        let via_short_circuit: Response =
            ApiRejection(ApiError::NotFound("record not found".to_owned())).into_response();

        assert_eq!(via_store.status(), StatusCode::NOT_FOUND);
        assert_eq!(via_short_circuit.status(), StatusCode::NOT_FOUND);

        let via_store_body = body_bytes(via_store).await;
        let via_short_circuit_body = body_bytes(via_short_circuit).await;
        assert_eq!(
            via_store_body, via_short_circuit_body,
            "the foreign-id 404 body must byte-equal the missing-id 404 body"
        );
    }

    /// A service principal must always get 404 on this route, never 401
    /// or 403. A service identity has no `app_user.id` of its own to
    /// enumerate, so the route cannot tell apart "your session",
    /// "another user's session", and "no such session" — exactly the
    /// same posture as [`SessionsDecision::Empty`] on the list endpoint.
    /// Returning 401/403 here would let a service caller probe the
    /// difference between "this id exists" and "this id doesn't" — the
    /// non-enumeration guarantee this route makes exists to close that
    /// gap.
    ///
    /// Reachable without a pool: the service-principal short-circuit
    /// runs before [`pool`], so `state_without_pool` is the right
    /// harness (the 503-on-no-pool check is covered by
    /// `every_database_backed_route_returns_503_without_a_pool` in
    /// `routes/identity.rs`'s tests, and this route is now under that
    /// same coverage umbrella).
    #[tokio::test]
    async fn delete_session_route_returns_404_for_a_service_principal() {
        let state = state_without_pool();
        let principal = AuthenticatedPrincipal(Principal {
            id: PrincipalId::Service(Uuid::new_v4()),
            tenant_ids: Vec::new(),
            display_name: "service-caller".to_owned(),
            permissions: lakehouse_auth::PermissionSet::default(),
            provider: "service".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        });
        let rejection = revoke_session(
            State(state),
            principal,
            Path("00000000-0000-0000-0000-000000000000".to_owned()),
        )
        .await
        .expect_err("a service principal must be a rejection, not a 2xx");
        let response = rejection.into_response();
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "a service principal must get 404, never 401/403"
        );
    }

    /// The read-after-write property of the route: after
    /// [`revoke_session`] returns Ok, the cookie for that session must
    /// be rejected by [`lakehouse_auth::session::validate_session`].
    /// If the DB write succeeded but the session still validated, the
    /// cookie has been cleared by the UI but the server still accepts
    /// the request — exactly the gap this task exists to close.
    ///
    /// This is the test that ties `lakehouse_auth::session::validate_session`'s
    /// own live predicate (`revoked_at IS NULL AND expires_at > now()`)
    /// to [`lakehouse_store::sessions::revoke_session_as_caller`]'s
    /// UPDATE — both predicate on the same column, so a write that
    /// passes our test passes theirs by construction. Asserting via
    /// `validate_session` (rather than reading `revoked_at` directly)
    /// is the property the route exists to provide end-to-end, and
    /// is what would fail if our UPDATE didn't actually flip
    /// `revoked_at` to non-`NULL`.
    #[sqlx::test(migrations = "../../migrations")]
    async fn delete_session_route_makes_validate_session_reject_the_same_token(
        pool: sqlx::PgPool,
    ) -> sqlx::Result<()> {
        use lakehouse_test_support as _;

        let user_id: Uuid = sqlx::query_scalar(
            "INSERT INTO app_user (name, username, email) VALUES ($1, $2, $3) RETURNING id",
        )
        .bind("caller")
        .bind("caller")
        .bind("caller@x.invalid")
        .fetch_one(&pool)
        .await?;

        let token = lakehouse_auth::session::create_session(
            &pool,
            user_id,
            lakehouse_auth::session::DEFAULT_SESSION_TTL,
            None,
            None,
            &[],
        )
        .await
        .expect("mint a session");

        let session_id: Uuid = sqlx::query_scalar(
            "SELECT id FROM session \
             WHERE app_user_id = $1 AND revoked_at IS NULL \
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(user_id)
        .fetch_one(&pool)
        .await?;

        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), database_url_for(&pool));
        let state = AppState::new(Config::from_map(&env).unwrap());

        let principal = AuthenticatedPrincipal(Principal {
            id: PrincipalId::User(user_id),
            tenant_ids: Vec::new(),
            display_name: "caller".to_owned(),
            permissions: lakehouse_auth::PermissionSet::default(),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        });

        let response = revoke_session(State(state), principal, Path(session_id.to_string()))
            .await
            .expect("revoke must succeed for the session's owner");
        assert_eq!(
            response.status(),
            StatusCode::NO_CONTENT,
            "a successful revoke returns 204"
        );

        let validation = lakehouse_auth::session::validate_session(&pool, &token).await;
        assert!(
            matches!(validation, Err(lakehouse_auth::AuthError::SessionInvalid)),
            "validate_session must reject the revoked token, got {validation:?}"
        );
        Ok(())
    }
}
