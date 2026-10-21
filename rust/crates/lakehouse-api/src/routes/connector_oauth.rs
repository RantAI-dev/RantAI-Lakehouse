//! Sign in to a REST or business-app (`saas`) connector with `OAuth2`
//! (`SRC-12` part C, decisions 6 to 8, 11 and 12; `SRC-3` AD8): the sign-in,
//! the provider's callback, the orchestrator's report of a rotated refresh
//! token, and sign-out.
//!
//! A `rest` connector names its own sign-in, token and revocation addresses
//! in its dial. A `saas` connector names none: they come from the vendor table
//! (`lakehouse_store::saas_vendors`), built from its `account`, so a dial
//! cannot send the client secret or the refresh token anywhere the table does
//! not (its parser refuses an unknown field). The vendor table also says how
//! the client authenticates at the token endpoint.
//!
//! # The routes
//!
//! A person with `connector:manage` on a connector of their own tenant (the
//! tenant gate is the route layer):
//!
//! - `GET  /api/connectors/{id}/oauth` -- is it signed in, the callback
//!   address to copy into the provider's app settings, whether a revocation
//!   address is set;
//! - `POST /api/connectors/{id}/oauth/start` -- the provider's sign-in address
//!   with a `state` bound to this connector and this user;
//! - `POST /api/connectors/{id}/oauth/sign-out` -- tell the provider to forget
//!   the refresh token (when it has a revocation address) and delete it.
//!
//! The provider's redirect, `GET /api/connectors/oauth/callback?code&state`,
//! needs a signed-in user and nothing more at the route layer (it has no
//! connector in its path); the handler does the rest of the checking.
//!
//! The orchestrator, on its `ingest:read` service identity (checked with
//! `schema_changes::require_orchestrator`, as the run reports are):
//!
//! - `POST /api/connectors/{id}/oauth/refresh-token` -- the provider rotated
//!   the refresh token during a load; the API seals the new one in place of the
//!   old one. The orchestrator never writes the credentials volume.
//!
//! # The callback's rules (hard requirements, each tested)
//!
//! 1. The `state` is found by its hash and consumed in the same statement
//!    (`lakehouse_store::connector_oauth::consume`): unknown, expired and
//!    replayed `state`s find nothing, and a refused callback has still burned
//!    its `state`.
//! 2. The signed-in user must be the one who started the sign-in, must still
//!    belong to the connector's tenant and hold `connector:manage`.
//! 3. Any refusal ends with a fixed page and stores nothing. The provider's
//!    `error`, `error_description` and the body of any answer are never read
//!    into a response, a log or a table.
//! 4. The code is exchanged with the PKCE verifier that never left the server
//!    and with `redirect_uri` from the operator's setting, not from the request.
//! 5. The refresh token is sealed as the connector's managed secondary
//!    credential (part A). An access token is never stored. Neither is ever
//!    logged or returned.

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use lakehouse_auth::{Principal, PrincipalId};
use lakehouse_core::ApiError;
use lakehouse_store::audit as store_audit;
use lakehouse_store::connector_oauth as store_oauth;
use lakehouse_store::connectors::{self, ConnectorDialInfo};
use lakehouse_store::ingest_spec::{Dial, RestAuth, SaasAuth};
use lakehouse_store::saas_vendors::{ClientAuth, SaasVendor};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::connectors::{
    connector_audit_event, parse_body, pool, remove_refresh_token, store_refresh_token,
};
use super::schema_changes::require_orchestrator;
use crate::connector_oauth::{self, TokenFailure};
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::state::AppState;

/// Shown for a connector that does not sign in with the authorization code.
const NOT_AN_OAUTH_CONNECTOR: &str =
    "This connector does not sign in with OAuth authorization code.";
/// Shown when the operator has not set the callback address.
const CALLBACK_NOT_SET: &str = "OAuth sign-in is not set up on this installation: the operator \
     must set CONNECTOR_OAUTH_REDIRECT_URI to this product's callback address.";
/// Shown when the client secret cannot be read at sign-in start.
const CLIENT_SECRET_MISSING: &str = "Save the connector's client secret first: sign-in needs it \
     to exchange the code.";
/// The longest `state` and `code` the callback looks at.
const MAX_STATE_LEN: usize = 256;
const MAX_CODE_LEN: usize = 4096;

/// The `OAuth2` settings of a connector's dial, copied out of
/// [`ConnectorDialInfo`].
struct App {
    client_id: String,
    authorize_url: String,
    token_url: String,
    revocation_url: Option<String>,
    scopes: Vec<String>,
    /// How the client proves itself at the token endpoint. Always the body
    /// for a `rest` connector; the vendor table's choice for a `saas` one.
    client_auth: ClientAuth,
    /// The vendor id of a `saas` connector, for the gate-only mock origin
    /// (`connector_oauth::gate_rebase`); `None` for a `rest` connector.
    gate_vendor: Option<&'static str>,
}

/// The authorization-code settings of `info`, or `None` for a connector that
/// is not a REST or business-app one with that auth kind.
fn app_of(info: &ConnectorDialInfo) -> Option<App> {
    match info.adapter.as_deref() {
        Some("rest") => rest_app_of(info),
        Some("saas") => saas_app_of(info),
        _ => None,
    }
}

/// The settings of a `saas` connector that signs in in the browser, from the
/// vendor table (`SRC-3` AD8). `None` for a dial that does not parse, one that
/// uses the vendor's integration credential, or a vendor not in the table.
fn saas_app_of(info: &ConnectorDialInfo) -> Option<App> {
    let Ok(Dial::Saas(dial)) = Dial::parse("saas", &info.dial) else {
        return None;
    };
    let SaasAuth::Oauth2AuthorizationCode { client_id } = dial.auth else {
        return None;
    };
    let vendor = lakehouse_store::saas_vendors::vendor(&dial.vendor)?;
    let parts = vendor.validate_account(&dial.account).ok()?;
    let render = |template: &str| SaasVendor::render(template, &parts);
    Some(App {
        client_id,
        authorize_url: render(vendor.oauth.authorize_url),
        token_url: render(vendor.oauth.token_url),
        revocation_url: vendor.oauth.revocation_url.map(render),
        scopes: vendor
            .oauth
            .scopes
            .iter()
            .map(|scope| render(scope))
            .collect(),
        client_auth: vendor.oauth.client_auth,
        gate_vendor: Some(vendor.id),
    })
}

/// The authorization-code settings of a `rest` connector.
fn rest_app_of(info: &ConnectorDialInfo) -> Option<App> {
    let Ok(Dial::Rest(dial)) = Dial::parse("rest", &info.dial) else {
        return None;
    };
    match dial.auth {
        RestAuth::Oauth2AuthorizationCode {
            client_id,
            authorize_url,
            token_url,
            revocation_url,
            scopes,
        } => Some(App {
            client_id,
            authorize_url,
            token_url,
            revocation_url,
            scopes,
            client_auth: ClientAuth::Post,
            gate_vendor: None,
        }),
        _ => None,
    }
}

async fn dial_info_of(state: &AppState, id: &str) -> Result<ConnectorDialInfo, ApiError> {
    connectors::get_connector_dial_info(pool(state)?, id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Connector {id} not found")))
}

fn require_user(principal: &Principal) -> Result<String, ApiError> {
    match principal.id {
        PrincipalId::User(id) => Ok(id.to_string()),
        PrincipalId::Service(_) => Err(ApiError::PermissionDenied(
            "only a signed-in person can sign a connector in".to_owned(),
        )),
    }
}

// ── status ─────────────────────────────────────────────────────────────────

/// `GET /api/connectors/{id}/oauth`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusResponse {
    /// Whether the connector holds a refresh token it can read.
    signed_in: bool,
    /// The address to enter in the provider's app settings; `null` when the
    /// operator has not set it (then Sign in is refused too).
    callback_url: Option<String>,
    /// Whether the dial names a revocation address, that is, whether signing
    /// out or deleting the connector can tell the provider.
    revocation_configured: bool,
}

/// `GET /api/connectors/{id}/oauth` -- what the connector page needs to draw
/// its Sign in panel. Holds no token and no secret.
///
/// # Errors
///
/// 404 for an unknown connector (or another tenant's, at the route layer); 409
/// for a connector that does not use `OAuth2` authorization code; 503/500 as
/// the store reports.
pub async fn status(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<StatusResponse>> {
    let info = dial_info_of(&state, &id).await?;
    let app = app_of(&info).ok_or_else(|| ApiError::Conflict(NOT_AN_OAUTH_CONNECTOR.to_owned()))?;
    Ok(ApiJson(StatusResponse {
        signed_in: refresh_token_of(&state, &info).await.is_some(),
        callback_url: state.config.connector_oauth_redirect_uri.clone(),
        revocation_configured: app.revocation_url.is_some(),
    }))
}

/// The stored refresh token, when there is one that opens.
async fn refresh_token_of(
    state: &AppState,
    info: &ConnectorDialInfo,
) -> Option<lakehouse_core::secret::SecretValue> {
    let secondary = info.secret_ref_secondary.as_deref()?;
    state
        .connector_secret_resolver
        .resolve_dyn(secondary)
        .await
        .ok()
}

// ── start ──────────────────────────────────────────────────────────────────

/// `POST /api/connectors/{id}/oauth/start` response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartResponse {
    /// The provider's sign-in page with this flow's query. The console opens
    /// it in a new window.
    authorize_url: String,
    /// How long the sign-in may take.
    expires_in_seconds: i32,
}

/// `POST /api/connectors/{id}/oauth/start` -- begin a sign-in.
///
/// The `state` and the PKCE verifier are 256 random bits each, from the
/// session-token generator. Only the hash of the `state` is stored, with the
/// connector, the user and the verifier; the response carries the `state` (the
/// provider echoes it back) and the verifier's S256 challenge, never the
/// verifier.
///
/// # Errors
///
/// 403 for a service identity; 404 for an unknown connector; 409 for a
/// connector that does not use authorization code, or one whose client secret
/// is not saved; 503 when the operator has not set the callback address or the
/// store is unavailable.
pub async fn start(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<StartResponse>> {
    let user = require_user(&principal)?;
    let info = dial_info_of(&state, &id).await?;
    let app = app_of(&info).ok_or_else(|| ApiError::Conflict(NOT_AN_OAUTH_CONNECTOR.to_owned()))?;
    let redirect_uri = state
        .config
        .connector_oauth_redirect_uri
        .as_deref()
        .ok_or_else(|| ApiError::Unavailable(CALLBACK_NOT_SET.to_owned()))?;
    // Fail before the person is sent to the provider: a sign-in that cannot be
    // finished for want of the client secret would only end in an error page.
    if state
        .connector_secret_resolver
        .resolve_dyn(&info.secret_ref)
        .await
        .is_err()
    {
        return Err(ApiError::Conflict(CLIENT_SECRET_MISSING.to_owned()).into());
    }

    let flow_state = lakehouse_auth::generate_opaque_token();
    let verifier = lakehouse_auth::generate_opaque_token();
    store_oauth::begin(
        pool(&state)?,
        &connector_oauth::hash_state(flow_state.expose()),
        &id,
        &user,
        verifier.expose(),
    )
    .await?;
    let authorize_url = connector_oauth::authorize_url(
        &app.authorize_url,
        &app.client_id,
        redirect_uri,
        &app.scopes,
        flow_state.expose(),
        &connector_oauth::pkce_challenge(verifier.expose()),
    )
    .ok_or_else(|| ApiError::BadRequest(NOT_AN_OAUTH_CONNECTOR.to_owned()))?;

    audit(&state, &principal, "connector.oauth_start", &id, json!({})).await;
    Ok(ApiJson(StartResponse {
        authorize_url,
        expires_in_seconds: store_oauth::STATE_TTL_SECONDS,
    }))
}

// ── callback ───────────────────────────────────────────────────────────────

/// The provider's redirect. `error` and `error_description` are never read:
/// the presence of `error` is all the handler uses.
#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// Why a callback ended without signing in. Each one renders as a fixed
/// sentence ([`Refusal::text`]); none carries anything the provider or the
/// request said.
#[derive(Debug, PartialEq, Eq)]
enum Refusal {
    /// No `state`, or one too long to be ours.
    Malformed,
    /// Unknown, expired or already used.
    UnknownState,
    /// Another signed-in user than the one who started it.
    WrongUser,
    /// The user may no longer manage this connector, or it is gone or changed.
    NotAllowed,
    /// The operator has not set the callback address.
    NotSetUp,
    /// The provider reported an error, or returned no code.
    ProviderDeclined,
    /// The client secret cannot be read.
    ClientSecret,
    /// The token request failed (the failure's own fixed text).
    Provider(TokenFailure),
    /// The provider answered but issued no refresh token.
    NoRefreshToken,
    /// The refresh token could not be stored, or the store was unavailable.
    Store,
}

impl Refusal {
    fn text(&self) -> String {
        match self {
            Self::Malformed => "This sign-in link is not valid. Start again from the connector page.".to_owned(),
            Self::UnknownState => "This sign-in link is unknown, has expired or was already used. Start again from the connector page.".to_owned(),
            Self::WrongUser => "This sign-in was started by someone else. Nothing was saved. Start again from your own session.".to_owned(),
            Self::NotAllowed => "You can no longer manage this connector, or it has changed. Nothing was saved.".to_owned(),
            Self::NotSetUp => CALLBACK_NOT_SET.to_owned(),
            Self::ProviderDeclined => "The provider did not complete the sign-in. Nothing was saved. Start again from the connector page.".to_owned(),
            Self::ClientSecret => CLIENT_SECRET_MISSING.to_owned(),
            Self::Provider(failure) => format!("{} Nothing was saved.", failure.message()),
            Self::NoRefreshToken => "The provider signed you in but did not issue a refresh token. Some providers need an offline scope (for example offline_access) in the connector's scopes. Nothing was saved.".to_owned(),
            Self::Store => "The sign-in succeeded but could not be saved. Nothing was saved. Try again, or ask the operator to check the credential store.".to_owned(),
        }
    }
}

/// `GET /api/connectors/oauth/callback?code&state` -- where the provider sends
/// the person back. Answers an HTML page with a fixed sentence and closes the
/// window; see the module docs for the rules.
pub async fn callback(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Query(query): Query<CallbackQuery>,
) -> Response {
    let outcome = finish_sign_in(&state, &principal, query).await;
    match outcome {
        Ok(connector_id) => {
            audit(
                &state,
                &principal,
                "connector.oauth_signed_in",
                &connector_id,
                json!({}),
            )
            .await;
            result_page(
                true,
                "Signed in. You can close this window and return to the connector page.",
            )
        }
        Err(refusal) => {
            tracing::warn!(?refusal, "OAuth callback refused");
            result_page(false, &refusal.text())
        }
    }
}

/// The token request of the code exchange, the client authenticating as the vendor
/// table says (PR #214 CI clippy: split out of [`finish_sign_in`], behaviour unchanged).
async fn exchange_code(
    state: &AppState,
    app: &App,
    client_secret: &lakehouse_core::secret::SecretValue,
    exchange: &[(&str, &str)],
) -> Result<connector_oauth::Tokens, TokenFailure> {
    let internal_hosts = state.config.connector_internal_hosts();
    // Slice 2 review SHOULD-FIX 5: the gate-only mock origin; a no-op unless the
    // gate-only switch is on, and never for a `rest` connector.
    let token_url = app.gate_vendor.map_or_else(
        || app.token_url.clone(),
        |vendor| {
            connector_oauth::gate_rebase(state.config.oauth_settings(), vendor, &app.token_url)
        },
    );
    match app.client_auth {
        ClientAuth::Post => {
            let mut form = exchange.to_vec();
            form.push(("client_id", app.client_id.as_str()));
            form.push(("client_secret", client_secret.expose_secret()));
            connector_oauth::token_request(
                &token_url,
                &form,
                &internal_hosts,
                state.config.oauth_settings(),
            )
            .await
        }
        // Slice 2 review BLOCKER 3 (`client_secret_post_json`): the same fields as
        // `Post`, as a JSON body (Zendesk's token endpoint requires it).
        ClientAuth::PostJson => {
            let mut form = exchange.to_vec();
            form.push(("client_id", app.client_id.as_str()));
            form.push(("client_secret", client_secret.expose_secret()));
            connector_oauth::token_request_json(
                &token_url,
                &form,
                &internal_hosts,
                state.config.oauth_settings(),
            )
            .await
        }
        // `SRC-3` AD8: the client is in the header, not the body.
        ClientAuth::Basic => {
            connector_oauth::token_request_basic(
                &token_url,
                exchange,
                &app.client_id,
                client_secret.expose_secret(),
                &internal_hosts,
                state.config.oauth_settings(),
            )
            .await
        }
    }
}

/// The callback's work. Returns the connector's id on success; on any error
/// nothing was stored.
async fn finish_sign_in(
    state: &AppState,
    principal: &Principal,
    query: CallbackQuery,
) -> Result<String, Refusal> {
    let Some(flow_state) = query
        .state
        .as_deref()
        .filter(|s| !s.is_empty() && s.len() <= MAX_STATE_LEN)
    else {
        return Err(Refusal::Malformed);
    };
    // First, and whatever follows: the `state` is spent.
    let pool = state.pg.as_deref().ok_or(Refusal::Store)?;
    let pending = store_oauth::consume(pool, &connector_oauth::hash_state(flow_state))
        .await
        .map_err(|_| Refusal::Store)?
        .ok_or(Refusal::UnknownState)?;

    let user = match principal.id {
        PrincipalId::User(id) => id.to_string(),
        PrincipalId::Service(_) => return Err(Refusal::WrongUser),
    };
    if pending.principal_id != user {
        return Err(Refusal::WrongUser);
    }
    // The route layer could not check these (the path names no connector).
    if !principal.has("connector:manage")
        || !connectors::connector_in_tenants(pool, &pending.connector_id, &principal.tenant_ids)
            .await
            .map_err(|_| Refusal::Store)?
    {
        return Err(Refusal::NotAllowed);
    }
    let code = match (&query.error, query.code.as_deref()) {
        (None, Some(code)) if !code.is_empty() && code.len() <= MAX_CODE_LEN => code,
        _ => return Err(Refusal::ProviderDeclined),
    };

    let info = connectors::get_connector_dial_info(pool, &pending.connector_id)
        .await
        .map_err(|_| Refusal::Store)?
        .ok_or(Refusal::NotAllowed)?;
    let app = app_of(&info).ok_or(Refusal::NotAllowed)?;
    let redirect_uri = state
        .config
        .connector_oauth_redirect_uri
        .as_deref()
        .ok_or(Refusal::NotSetUp)?;
    let client_secret = state
        .connector_secret_resolver
        .resolve_dyn(&info.secret_ref)
        .await
        .map_err(|_| Refusal::ClientSecret)?;

    let exchange = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("code_verifier", pending.code_verifier.as_str()),
    ];
    let tokens = exchange_code(state, &app, &client_secret, &exchange)
        .await
        .map_err(Refusal::Provider)?;
    // The access token ends here: it is not needed to sign in and is never
    // stored.
    drop(tokens.access_token);
    let refresh_token = tokens.refresh_token.ok_or(Refusal::NoRefreshToken)?;

    store_refresh_token(
        state,
        &pending.connector_id,
        info.secret_ref_secondary.as_deref(),
        refresh_token,
    )
    .await
    .map_err(|_| Refusal::Store)?;
    Ok(pending.connector_id)
}

/// The fixed page the person's window ends on. `message` is always one of this
/// module's constants, never request or provider text, so it is written into
/// the page as it is. The script (allowed by a one-time nonce; nothing else
/// may run) tells the console window and closes this one.
fn result_page(ok: bool, message: &str) -> Response {
    let nonce = lakehouse_auth::generate_opaque_token();
    let nonce = nonce.expose();
    let title = if ok {
        "Signed in"
    } else {
        "Sign-in not completed"
    };
    let body = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>{title}</title>\
         <style>body{{font:16px system-ui,sans-serif;margin:3rem auto;max-width:34rem;padding:0 1rem}}</style>\
         </head><body><h1>{title}</h1><p>{message}</p>\
         <script nonce=\"{nonce}\">try{{if(window.opener){{window.opener.postMessage(\
         {{type:\"lakehouse-oauth\",ok:{ok}}},window.location.origin)}}}}catch(e){{}}\
         setTimeout(function(){{window.close()}},{close_after})</script></body></html>",
        close_after = if ok { 1500 } else { 8000 },
    );
    let mut response = (StatusCode::OK, body).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    if let Ok(csp) = HeaderValue::from_str(&format!(
        "default-src 'none'; style-src 'unsafe-inline'; script-src 'nonce-{nonce}'; \
         frame-ancestors 'none'"
    )) {
        headers.insert(header::CONTENT_SECURITY_POLICY, csp);
    }
    response
}

// ── the orchestrator's report ──────────────────────────────────────────────

/// The `POST …/oauth/refresh-token` body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RefreshTokenBody {
    /// The new refresh token. Write-only: [`super::connectors::SecretInput`]'s
    /// `Debug` never prints it.
    refresh_token: super::connectors::SecretInput,
}

/// The response: the token is never echoed.
#[derive(Debug, Serialize)]
pub struct StoredResponse {
    stored: bool,
}

/// `POST /api/connectors/{id}/oauth/refresh-token` -- the provider issued a new
/// refresh token while the orchestrator used the old one (`SRC-12` decision 7).
/// The API seals it in place of the old one.
///
/// # Errors
///
/// 403 for a caller that is neither the orchestrator's service identity nor an
/// unrestricted administrator; 400 for an invalid body or token; 404 for an
/// unknown connector; 409 for a connector whose auth kind is not `OAuth2`
/// authorization code (this route is for no other kind); 503/500 as the
/// credential store and the database report (fixed texts).
pub async fn update_refresh_token(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<ApiJson<StoredResponse>> {
    require_orchestrator(&principal)?;
    let body: RefreshTokenBody = parse_body(&body)?;
    let token = body.refresh_token.into_secret();
    crate::connector_secret_store::validate_secret_value(token.expose_secret())
        .map_err(|_| ApiError::BadRequest("refreshToken is not a usable token".to_owned()))?;
    let info = dial_info_of(&state, &id).await?;
    if app_of(&info).is_none() {
        return Err(ApiError::Conflict(NOT_AN_OAUTH_CONNECTOR.to_owned()).into());
    }
    store_refresh_token(&state, &id, info.secret_ref_secondary.as_deref(), token).await?;
    audit(
        &state,
        &principal,
        "connector.oauth_refresh_token_rotated",
        &id,
        json!({}),
    )
    .await;
    Ok(ApiJson(StoredResponse { stored: true }))
}

// ── sign out and delete ────────────────────────────────────────────────────

/// What happened to the provider's copy of the token when a connector let it
/// go (`SRC-12` decision 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum Revocation {
    /// Not an `OAuth2` authorization-code connector.
    NotApplicable,
    /// The dial names no revocation address: the provider cannot be told, and
    /// the token stays valid there until it expires.
    NotConfigured,
    /// There was no refresh token to revoke.
    NoToken,
    /// The provider accepted the revocation.
    Done,
    /// The provider could not be told (refused, unreachable, or the address was
    /// not allowed); the token may stay valid there until it expires.
    Failed,
}

impl Revocation {
    fn message(self) -> &'static str {
        match self {
            Self::NotApplicable => "",
            Self::NotConfigured => {
                "The provider was not told: this connector has no revocation address, so the token stays valid at the provider until it expires."
            }
            Self::NoToken => "There was no token to revoke.",
            Self::Done => "The provider was told to revoke the token.",
            Self::Failed => {
                "The provider could not be told to revoke the token; it may stay valid there until it expires."
            }
        }
    }
}

/// Tell the provider to revoke the connector's refresh token, if the dial has a
/// revocation address and a token is stored. Best effort: the caller removes the
/// local copy whatever this returns. Used by sign-out and by deleting the
/// connector, before the credential files go.
pub(super) async fn revoke_stored_token(state: &AppState, info: &ConnectorDialInfo) -> Revocation {
    let Some(app) = app_of(info) else {
        return Revocation::NotApplicable;
    };
    let Some(url) = app.revocation_url.as_deref() else {
        return Revocation::NotConfigured;
    };
    let Some(token) = refresh_token_of(state, info).await else {
        return Revocation::NoToken;
    };
    let Ok(client_secret) = state
        .connector_secret_resolver
        .resolve_dyn(&info.secret_ref)
        .await
    else {
        return Revocation::Failed;
    };
    match connector_oauth::revoke_request(
        url,
        &token,
        &app.client_id,
        &client_secret,
        app.client_auth,
        &state.config.connector_internal_hosts(),
        state.config.oauth_settings(),
    )
    .await
    {
        Ok(()) => Revocation::Done,
        Err(failure) => {
            tracing::warn!(?failure, "OAuth revocation failed");
            Revocation::Failed
        }
    }
}

/// `POST /api/connectors/{id}/oauth/sign-out` response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignOutResponse {
    /// Always `true`: the local token is gone.
    signed_out: bool,
    /// What happened at the provider.
    revocation: Revocation,
    /// Says it in words, for the page.
    message: &'static str,
}

/// `POST /api/connectors/{id}/oauth/sign-out` -- revoke the token at the
/// provider when the dial has a revocation address, then delete the stored
/// refresh token. The local token is deleted even when the provider could not
/// be told, and the response says which it was (decision 8).
///
/// # Errors
///
/// 404 for an unknown connector; 409 for a connector that does not use
/// `OAuth2` authorization code; 503/500 as the store reports.
pub async fn sign_out(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<SignOutResponse>> {
    let info = dial_info_of(&state, &id).await?;
    if app_of(&info).is_none() {
        return Err(ApiError::Conflict(NOT_AN_OAUTH_CONNECTOR.to_owned()).into());
    }
    let revocation = revoke_stored_token(&state, &info).await;
    remove_refresh_token(&state, &id, info.secret_ref_secondary.as_deref()).await;
    audit(
        &state,
        &principal,
        "connector.oauth_signed_out",
        &id,
        json!({ "revocation": revocation }),
    )
    .await;
    Ok(ApiJson(SignOutResponse {
        signed_out: true,
        revocation,
        message: revocation.message(),
    }))
}

/// Best-effort audit, as every route of the connector module writes it:
/// the action and the connector, never a token, a `state` or a code.
async fn audit(
    state: &AppState,
    principal: &Principal,
    action: &str,
    connector_id: &str,
    args: serde_json::Value,
) {
    let event = connector_audit_event(principal, action, connector_id, args, "executed");
    if let Some(pool) = state.pg.as_deref()
        && let Err(err) = store_audit::insert(pool, event).await
    {
        tracing::warn!(%err, connector_id = %connector_id, action, "failed to record an OAuth audit event");
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn info(adapter: &str, dial: serde_json::Value) -> ConnectorDialInfo {
        ConnectorDialInfo {
            kind: "Zendesk".to_owned(),
            host: String::new(),
            secret_ref: "env:UNUSED".to_owned(),
            secret_ref_secondary: None,
            adapter: Some(adapter.to_owned()),
            dial,
        }
    }

    fn oauth_dial(vendor: &str, account: &str) -> serde_json::Value {
        json!({
            "vendor": vendor, "account": account,
            "auth": {"type": "oauth2_authorization_code", "clientId": "app-1"}
        })
    }

    #[test]
    fn a_saas_connectors_addresses_and_scopes_come_from_the_vendor_table() {
        let app = app_of(&info("saas", oauth_dial("zendesk", "Acme"))).unwrap();
        assert_eq!(app.client_id, "app-1");
        assert_eq!(
            app.authorize_url,
            "https://acme.zendesk.com/oauth/authorizations/new"
        );
        assert_eq!(app.token_url, "https://acme.zendesk.com/oauth/tokens");
        assert_eq!(app.revocation_url, None);
        assert_eq!(app.scopes, ["read"]);
        assert_eq!(app.client_auth, ClientAuth::PostJson);

        let sf = app_of(&info("saas", oauth_dial("salesforce", "acme"))).unwrap();
        assert_eq!(
            sf.revocation_url.as_deref(),
            Some("https://acme.my.salesforce.com/services/oauth2/revoke")
        );
        let ns = app_of(&info("saas", oauth_dial("netsuite", "1234567_SB1"))).unwrap();
        assert_eq!(ns.client_auth, ClientAuth::Basic);
        assert_eq!(
            ns.token_url,
            "https://1234567-sb1.suitetalk.api.netsuite.com/services/rest/auth/oauth2/v1/token"
        );
        let dyn365 = app_of(&info(
            "saas",
            oauth_dial("dynamics365", "org.crm4.dynamics.com"),
        ))
        .unwrap();
        assert_eq!(
            dyn365.scopes,
            [
                "https://org.crm4.dynamics.com/user_impersonation",
                "offline_access"
            ]
        );
    }

    #[test]
    fn a_saas_dial_cannot_override_a_vendor_address() {
        let mut dial = oauth_dial("zendesk", "acme");
        dial["auth"]["tokenUrl"] = json!("https://evil.example.invalid/token");
        assert!(app_of(&info("saas", dial)).is_none());
        let mut dial = oauth_dial("zendesk", "acme");
        dial["auth"]["authorizeUrl"] = json!("https://evil.example.invalid/authorize");
        assert!(app_of(&info("saas", dial)).is_none());
        // The account cannot carry a host either.
        assert!(app_of(&info("saas", oauth_dial("zendesk", "evil.example.invalid"))).is_none());
    }

    #[test]
    fn an_integration_credential_or_another_adapter_is_not_a_sign_in() {
        let token = json!({"vendor": "hubspot", "account": "123", "auth": {"type": "integration"}});
        assert!(app_of(&info("saas", token)).is_none());
        assert!(app_of(&info("sql", oauth_dial("zendesk", "acme"))).is_none());
    }

    #[test]
    fn the_sign_in_address_of_a_saas_connector_is_built_from_its_app() {
        let app = app_of(&info("saas", oauth_dial("confluence", "acme"))).unwrap();
        let url = connector_oauth::authorize_url(
            &app.authorize_url,
            &app.client_id,
            "https://console.example.invalid/api/connectors/oauth/callback",
            &app.scopes,
            "state-1",
            "challenge-1",
        )
        .unwrap();
        assert!(
            url.starts_with("https://auth.atlassian.com/authorize?"),
            "{url}"
        );
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("offline_access"));
    }
}
