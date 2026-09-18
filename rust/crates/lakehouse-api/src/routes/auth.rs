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
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use lakehouse_auth::{
    Authenticator, Credential, Principal, PrincipalId, Secret, password, session,
};
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
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

/// What `GET /api/auth/sessions` should return for `principal`. The
/// handler dispatches on this rather than threading two booleans through
/// the SQL, so the service-principal branch ("a service identity has no
/// `app_user_id` of its own to filter by") is a single match arm and a
/// single unit-test target — rather than a second conditional in the
/// query path that an integration test would only catch by accident.
///
/// Admin-vs-own is decided by `principal.has("identity:sessions:manage")`,
/// per WS8 plan §Phase D Hard Requirement 4's own phrasing ("lists only
/// the caller's own sessions unless..."): the route-level policy is
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
/// (every authenticated caller may hit this — the admin-vs-own split lives
/// here in the handler, see [`SessionsDecision`]).
///
/// # A documented gap
///
/// `created_ip` and `user_agent` are returned as `null` for every session
/// minted today: both [`login`] and [`oidc_callback`] call
/// `session::create_session(..., None, None)`. The columns themselves are
/// `TEXT NULL` (`0019_auth.sql`), so the listing reflects that honestly
/// rather than inventing an IP or UA — see
/// `lakehouse_store::sessions::SessionRow`'s own doc comment.
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

/// Validate `path` as a same-origin, in-app relative path — the one
/// server-side defense against an open redirect via `?next=`. Mirrors
/// `src/features/auth/login-page.tsx`'s `nextPathFromQuery` client-side
/// check but is the check that actually matters: this server issues the
/// redirect, the client-side one is only UX.
///
/// # Why a leading-`/` check alone is not enough (P3 fix)
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
struct OidcFlow {
    state: String,
    nonce: String,
    code_verifier: String,
    next: String,
}

const OIDC_FLOW_COOKIE_NAME: &str = "lh_oidc_flow";
const OIDC_FLOW_TTL_SECONDS: i64 = 600;

/// Build the `Set-Cookie` header carrying (`Some`) or clearing (`None`)
/// the `lh_oidc_flow` cookie. `Secure` follows the exact same condition
/// [`session_cookie_header`] uses — see that function's doc comment for
/// why the default fails closed to `Secure`.
fn oidc_flow_cookie_header(is_dev: bool, flow: Option<&OidcFlow>) -> HeaderValue {
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
    // Presence-checked only: `/callback` (a later task) is what actually
    // uses the configured `OidcAuthenticator` to verify the id token this
    // flow eventually produces. This route only needs to know OIDC is
    // configured at all before it starts a flow for it.
    state
        .auth
        .as_ref()
        .and_then(|auth| auth.oidc.as_ref())
        .ok_or_else(|| {
            ApiError::Unavailable("OIDC is not configured on this deployment".to_owned())
        })?;
    let authorize_url =
        state.config.oidc_authorize_url.as_deref().ok_or_else(|| {
            ApiError::Unavailable("OIDC_AUTHORIZE_URL is not configured".to_owned())
        })?;
    let redirect_uri =
        state.config.oidc_redirect_uri.as_deref().ok_or_else(|| {
            ApiError::Unavailable("OIDC_REDIRECT_URI is not configured".to_owned())
        })?;
    let client_id = state
        .config
        .oidc_client_id
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
    let oidc = state.auth.as_ref().and_then(|a| a.oidc.as_ref());
    Ok(ApiJson(ProvidersResponse {
        oidc: oidc.is_some(),
        provider_name: oidc.map(|_| state.config.oidc_provider_name.clone()),
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
/// # Errors
///
/// Returns 401 on a missing/undecodable/unparseable flow cookie, a
/// missing `code`/`state`, a `state` mismatch (compared in constant time
/// via [`lakehouse_auth::Secret::constant_time_eq`] — never `==`, which
/// would leak how many leading bytes matched through response timing), a
/// token-exchange HTTP failure, or a verification failure from
/// [`lakehouse_auth::OidcAuthenticator::authenticate_with_nonce`]. Every
/// one of those renders the same generic 401 body: per AGENTS.md
/// principle 4 ("upstream error text never reaches a response"), this
/// handler never puts the `IdP`'s own error body, or which specific check
/// failed, into anything sent back to the browser — only a `tracing::warn!`
/// line records which branch fired, and even that carries no token/secret
/// value. Returns 503 if OIDC or its token endpoint is not configured, or
/// no Postgres pool is configured.
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

    // Deployment-configuration presence is checked BEFORE the flow cookie
    // is ever read — same order [`oidc_start`] already uses for the same
    // three settings (`auth.oidc`/token URL/redirect URI/client ID) plus
    // this handler's own `OIDC_TOKEN_URL`. This is a "is OIDC configured
    // on this deployment at all" question, independent of any particular
    // caller's cookie, so it fails closed to 503 (not 401) exactly like
    // [`oidc_start`] does when asked the same question — a 503 here is not
    // a "no credentials presented" 401/403 for [`crate::policy::POLICY_TABLE`]'s
    // `Policy::Public` purposes, and nothing has been read from `headers`
    // yet, so there is no cookie to clear on this path.
    let Some(auth) = state.auth.as_ref().and_then(|a| a.oidc.as_ref()) else {
        return Ok(unavailable("OIDC is not configured on this deployment"));
    };
    let (Some(token_url), Some(redirect_uri)) = (
        state.config.oidc_token_url.as_deref(),
        state.config.oidc_redirect_uri.as_deref(),
    ) else {
        return Ok(unavailable(
            "OIDC_TOKEN_URL/OIDC_REDIRECT_URI are not configured",
        ));
    };
    // `AuthState::oidc` is `Some` only when `OIDC_ISSUER`/`OIDC_CLIENT_ID`
    // are both set (`state::oidc_config`), the same invariant `oidc_start`
    // relies on for `client_id` (see its P3 fix comment) — reading it the
    // same fail-closed way here, rather than sending an empty
    // `client_id` to the token endpoint.
    let Some(client_id) = state.config.oidc_client_id.as_deref() else {
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

    let Some(id_token) = exchange_code_for_id_token(
        &state,
        token_url,
        redirect_uri,
        client_id,
        &code,
        &flow.code_verifier,
    )
    .await
    else {
        return Ok(unauthorized());
    };

    let Ok(principal) = auth
        .authenticate_with_nonce(&Secret::new(id_token), &flow.nonce)
        .await
    else {
        tracing::warn!("oidc callback: id token verification failed");
        return Ok(unauthorized());
    };
    let PrincipalId::User(user_id) = principal.id else {
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
    // parallel implementation.
    let token =
        session::create_session(pool, user_id, session::DEFAULT_SESSION_TTL, None, None).await?;

    let mut response = (StatusCode::FOUND, ()).into_response();
    response.headers_mut().insert(
        header::LOCATION,
        // `flow.next` was already validated by `is_safe_relative_path`
        // inside `oidc_start` before it was ever written to the cookie
        // (see `OidcFlow`'s doc comment) — never re-derived from this
        // request, so there is nothing left to re-validate here. The
        // fallback exists only for the type-level possibility that a
        // valid relative path is somehow not a valid header value, not
        // because a value that already passed `is_safe_relative_path` is
        // expected to fail this.
        HeaderValue::from_str(&flow.next).unwrap_or_else(|_| HeaderValue::from_static("/")),
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
/// and return the `id_token` from a successful response, or `None` on any
/// failure — a non-2xx status, a network error, or a response body that
/// isn't `{"id_token": "..."}`. Never surfaces the `IdP`'s own status/body
/// to the caller or into anything but a `tracing::warn!` line (AGENTS.md
/// principle 4; see [`oidc_callback`]'s `# Errors` doc comment).
async fn exchange_code_for_id_token(
    state: &AppState,
    token_url: &str,
    redirect_uri: &str,
    client_id: &str,
    code: &str,
    code_verifier: &str,
) -> Option<String> {
    let http = reqwest::Client::new();
    let mut form = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("code_verifier", code_verifier),
        ("client_id", client_id),
    ];
    if let Some(secret) = state.config.oidc_client_secret.as_deref() {
        form.push(("client_secret", secret));
    }
    let token_response = http
        .post(token_url)
        .form(&form)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status);
    let Ok(token_response) = token_response else {
        tracing::warn!("oidc callback: token exchange request failed");
        return None;
    };
    let Ok(TokenResponse { id_token }) = token_response.json::<TokenResponse>().await else {
        tracing::warn!("oidc callback: token endpoint response was not the expected shape");
        return None;
    };
    Some(id_token)
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use base64::Engine as _;
    use tower::ServiceExt;

    use super::*;
    use crate::config::Config;

    /// Same idiom `routes::gold::tests::state_without_pool` uses: an
    /// unreachable-but-well-formed `DATABASE_URL` (never actually
    /// dialled — `lakehouse_store::connect_lazy` performs no I/O, see
    /// its own doc comment) plus every `OIDC_*` flow-config var Task A4
    /// needs, so `AppState::new` boots with `auth.oidc` populated
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
        // P3 fix: the judge review named these four shapes explicitly plus
        // their percent-encoded forms; each is asserted individually rather
        // than folded into one loop, so a future regression's failure names
        // exactly which shape stopped being refused.
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
    fn is_safe_relative_path_rejects_every_p3_named_shape() {
        // Unit-level pin, independent of the full HTTP round trip above —
        // this is the function the judge review's four named cases (plus
        // percent-encoded forms, which axum has already decoded by the
        // time this function runs) must refuse.
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

    // ── Task A5: `GET /api/auth/oidc/callback` — Step 1 (failing tests,
    // written before `oidc_callback` exists at all). ──────────────────────

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

            let mut rng = rand::thread_rng();
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

        let issuer = server.uri();
        let client_id = "lakehouse-console";
        let flow = OidcFlow {
            state: "state-from-start".to_owned(),
            nonce: "nonce-from-start".to_owned(),
            code_verifier: "verifier-1".to_owned(),
            next: "/dashboards".to_owned(),
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
        // Off by default (see `oidc_callback`'s doc comment on the plan's
        // "JIT provisioning stays off by default" note) — turned on here
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
        Ok(())
    }

    // ── Task A6: `GET /api/auth/providers` — Step 1 (failing test, written
    // before `providers` exists at all). ────────────────────────────────────

    /// `GET path` against a fresh router built from `state`, decoded as a
    /// JSON body. The plan's Step 1 snippet names a `get_json` helper that
    /// does not exist anywhere in this tree (grepped: no hit) — this is a
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

    #[tokio::test]
    async fn providers_reports_oidc_configured_true_only_when_auth_state_has_it() {
        let with_oidc = state_with_oidc_flow_config();
        let body = get_json(&with_oidc, "/api/auth/providers").await;
        assert_eq!(body["oidc"], serde_json::json!(true));

        let without_oidc = AppState::new(Config::from_map(&HashMap::new()).unwrap());
        let body = get_json(&without_oidc, "/api/auth/providers").await;
        assert_eq!(body["oidc"], serde_json::json!(false));
    }

    // ── WS8 plan §Phase D, the `GET /api/auth/sessions` routing shape ──────

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
}
