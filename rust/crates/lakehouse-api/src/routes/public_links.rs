//! Public links to dashboards (`BI-19` part A; plan
//! `docs/superpowers/plans/2026-10-18-bi-19-sharing.md`, decisions D1 to D5;
//! feature page `docs/core/features/sharing-and-delivery.md`, decisions A1 to
//! A8).
//!
//! # Routes
//!
//! | Route | Policy | What |
//! | --- | --- | --- |
//! | `GET /api/dashboard/public-link?board=` | `dashboard:write` | the link of a dashboard, for the Share dialog |
//! | `PUT /api/dashboard/public-link` | `dashboard:write` | make a link, or change its expiry or password |
//! | `DELETE /api/dashboard/public-link?board=` | `dashboard:write` | end it |
//! | `POST /api/public/dashboard/{token}/unlock` | public | trade the password for a ticket |
//!
//! Reading the dashboard itself (`GET /api/public/dashboard/{token}`) is in
//! `routes::embed` and calls [`resolve`] and [`check_ticket`] from here, so a
//! link is judged by one function wherever it is opened. Part B of the plan
//! moves the three management routes from `dashboard:write` to "manage on the
//! dashboard".
//!
//! # What a visitor can tell
//!
//! An unknown, an expired and a switched-off link, a link whose dashboard is in
//! the trash or in a personal space, and a link while the administrator's
//! switch is off all answer the same `404 {"error":"not_found"}` (feature A5):
//! nobody can use the page to find out which links exist or ever did. A link
//! with a password and no valid ticket answers `401 password_required` with no
//! dashboard name and no data. A wrong password answers `401 wrong_password`; a locked link
//! answers `401 locked` (feature A4, as changed on 2026-10-24).
//!
//! # Passwords
//!
//! Stored only as an `Argon2id` hash made by `lakehouse_auth::password`, checked
//! by the same crate's verifier (constant-time by construction), never returned,
//! never logged, never in the audit record. A wrong try is counted on the
//! login lock's table under the key `public-link:<board id>` (five tries in
//! fifteen minutes lock the link's password page for fifteen; a try is counted
//! before it is checked, so simultaneous guesses cannot get past the limit), so the lock follows the
//! dashboard and survives a guesser asking for a different token; a new link
//! for the dashboard starts from a clean count.
//!
//! # Errors
//!
//! SEC-11: no store or `ClickHouse` text reaches a response; every failure goes
//! through `upstream_error`, and every sentence here is our own.

use std::time::Duration as StdDuration;

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use lakehouse_auth::throttle::{self, ThrottlePolicy};
use lakehouse_auth::{Principal, PrincipalId, Secret};
use lakehouse_bi::store::{self, Board};
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::public_links::{self, PasswordChange, PublicLink};
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};

use crate::error::{ApiRejection, ApiResult};
use crate::json::ApiJson;
use crate::routes::dashboard_organise::viewer_of;
use crate::state::AppState;
use crate::upstream_error;

/// The header a visitor sends with the ticket the unlock route gave them.
pub(crate) const TICKET_HEADER: &str = "x-public-link-ticket";

/// A password is 8 to 128 characters (feature A3; the top is only a bound on
/// the work one request can ask the hasher to do).
const MIN_PASSWORD_CHARS: usize = 8;
const MAX_PASSWORD_CHARS: usize = 128;

/// An expiry is between one hour and a year from now (feature A1).
const MIN_EXPIRY: Duration = Duration::hours(1);
const MAX_EXPIRY: Duration = Duration::days(365);

/// Five wrong passwords on one link within 15 minutes lock its password page
/// for 15 minutes (owner's answer 8, feature A4). Fixed, not the sign-in's
/// environment settings: a link's lock is the owner's promise to a customer.
const LOCK_FAILURES: u32 = 5;
const LOCK_MINUTES: i64 = 15;

const NEEDS_DATABASE: &str = "Public links need the database.";
const SWITCHED_OFF: &str = "Public links are switched off by an administrator.";
const EXPIRY_REFUSED: &str = "The expiry must be between 1 hour and 365 days from now.";
const PASSWORD_REFUSED: &str = "A password needs 8 to 128 characters.";
const BODY_SHAPE: &str = "body must be {\"board\": string, \"expiresAt\": \"<RFC 3339 time>\", \"password\"?: string | null, \"newLink\"?: boolean}";
const BOARD_NOT_FOUND: &str = "dashboard not found.";
const AUDIT_DOWN: &str = "the audit store is unavailable";

/// How many times the start-up copy tries before it gives up until the next
/// start, and how long it waits between tries. `ClickHouse` is often not ready
/// for the first seconds after a deployment starts; a copy that ran once at that
/// moment would leave every old link unreadable until somebody restarted the
/// API.
const COPY_TRIES: u32 = 20;
const COPY_WAIT: StdDuration = StdDuration::from_secs(15);

// ── Start-up copy (T1) ───────────────────────────────────────────────────

/// Copy the public links that live dashboards carried in their own row into
/// `dashboard_public_link`, once per database, each expiring 30 days from the
/// copy (D2, feature A2).
///
/// Runs in the background so a slow `ClickHouse` does not hold the API back.
/// Fail closed: until the copy has run, an old link answers "does not work";
/// it is never served from the old column. Without a Postgres pool it logs once
/// and does nothing (links need the database). The copy is idempotent and
/// safe on several API processes (`lakehouse_store::public_links::copy_legacy`
/// writes a marker in the same transaction), so every replica may run it.
pub fn spawn_legacy_copy(state: &AppState) {
    let Some(pool) = state.pg.clone() else {
        tracing::warn!("no Postgres pool configured; public links are unavailable");
        return;
    };
    let ch = state.clickhouse.clone();
    tokio::spawn(async move {
        for attempt in 1..=COPY_TRIES {
            let old = match store::legacy_public_links(&ch).await {
                Ok(old) => old,
                Err(err) => {
                    let _logged = upstream_error::report_ch(&upstream_error::DATABASE, &err);
                    tracing::warn!(attempt, "public links could not be read for the copy");
                    tokio::time::sleep(COPY_WAIT).await;
                    continue;
                }
            };
            match public_links::copy_legacy(&pool, &old).await {
                Ok(copied) => {
                    tracing::info!(copied, "public links copied to Postgres");
                    return;
                }
                Err(err) => {
                    tracing::warn!(attempt, %err, "public links could not be copied");
                    tokio::time::sleep(COPY_WAIT).await;
                }
            }
        }
        tracing::error!("public links were not copied; old links stay off until the next start");
    });
}

// ── Reading a link (D3) ──────────────────────────────────────────────────

/// A link that works, and the dashboard it opens.
pub(crate) struct Open {
    pub link: PublicLink,
    pub board: Board,
}

/// What a token resolves to.
pub(crate) enum Resolved {
    /// The link has not expired, the administrator's switch is on and the
    /// dashboard is live and outside the personal space.
    Open(Box<Open>),
    /// Anything else. The caller cannot tell which (feature A5).
    Gone,
}

/// `404 {"error":"not_found"}`: the one answer for a link that does not work.
pub(crate) fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        ApiJson(json!({ "error": "not_found" })),
    )
        .into_response()
}

fn pool_of(state: &AppState) -> Result<&PgPool, ApiError> {
    state
        .pg
        .as_deref()
        .ok_or_else(|| ApiError::Unavailable(NEEDS_DATABASE.to_owned()))
}

fn store_failure(err: &lakehouse_store::StoreError) -> ApiError {
    upstream_error::internal_error(&upstream_error::STORE, err)
}

/// Judge a token: the link, the administrator's switch, then the dashboard.
///
/// Fail closed: with no Postgres pool nothing resolves (a 503, not a 404, since
/// this is an outage and not a link that does not work).
///
/// # Errors
///
/// `503` without a database; otherwise the classified store error. No store
/// or `ClickHouse` text is in it.
pub(crate) async fn resolve(state: &AppState, token: &str) -> Result<Resolved, ApiError> {
    let pool = pool_of(state)?;
    let token = token.trim();
    if token.is_empty() {
        return Ok(Resolved::Gone);
    }
    if !crate::routes::settings::format_settings(state)
        .await?
        .public_links_enabled
    {
        return Ok(Resolved::Gone);
    }
    let Some(link) = public_links::get_live_by_token(pool, token)
        .await
        .map_err(|err| store_failure(&err))?
    else {
        return Ok(Resolved::Gone);
    };
    // The anonymous viewer cannot see a personal-space board, even if a link
    // was issued before it was moved there (`move_board` refuses that move;
    // this is the second lock). A board in the trash is not live.
    let board = store::get_board(
        &state.clickhouse,
        &link.board_id,
        &lakehouse_bi::access::Viewer::anonymous(),
    )
    .await
    .map_err(|err| {
        upstream_error::ch_error_as(
            &upstream_error::DATABASE,
            &err,
            upstream_error::FailedAs::Internal,
        )
    })?;
    Ok(board.map_or(Resolved::Gone, |board| {
        Resolved::Open(Box::new(Open { link, board }))
    }))
}

/// For a link with a password: `None` when the request carries a live ticket
/// for it, otherwise the `401 password_required` that tells nothing but that a
/// password is needed. A link without a password is always `None`.
pub(crate) async fn check_ticket(
    state: &AppState,
    link: &PublicLink,
    headers: &HeaderMap,
) -> Option<Response> {
    if !link.has_password() {
        return None;
    }
    let required = || {
        Some(
            (
                StatusCode::UNAUTHORIZED,
                ApiJson(json!({ "error": "password_required" })),
            )
                .into_response(),
        )
    };
    let Some(ticket) = headers
        .get(TICKET_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|t| !t.is_empty())
    else {
        return required();
    };
    let Ok(pool) = pool_of(state) else {
        return required();
    };
    let hash = lakehouse_auth::hash_token(&Secret::new(ticket));
    match public_links::ticket_is_live(pool, &link.board_id, &hash).await {
        Ok(true) => None,
        Ok(false) => required(),
        Err(err) => Some(ApiRejection(store_failure(&err)).into_response()),
    }
}

// ── Unlocking (D4) ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct UnlockBody {
    password: String,
}

/// The lock's policy, built from the constants above.
fn lock_policy() -> ThrottlePolicy {
    ThrottlePolicy {
        max_failures: LOCK_FAILURES,
        window: Duration::minutes(LOCK_MINUTES),
        lockout: Duration::minutes(LOCK_MINUTES),
    }
}

/// The key a link's tries are counted under.
///
/// BI-19 part A review fix (SHOULD-FIX) R2: not `key_for("public-link:<id>")`.
/// That is the hash a login for the email `public-link:<id>` produces, so
/// someone could lock a link by failing logins as that name (and a link's tries
/// would count as that name's). A login key is 64 hex characters from
/// `key_for`; this one is the prefix and the hash of the board id, which no
/// login can produce. Board ids are lower-case hex after `b_`, so `key_for`'s
/// lower-casing merges no two of them (plan section 5).
fn lock_key(board_id: &str) -> String {
    format!("public-link:{}", throttle::key_for(board_id))
}

fn wrong_password() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        ApiJson(json!({ "error": "wrong_password" })),
    )
        .into_response()
}

/// `401 {"error":"locked"}`: the link's password page is locked. BI-19 part A
/// QA fix P2 (decision A4 as changed by the owner on 2026-10-24): a locked link
/// says so, because a visitor with the right password could not otherwise tell
/// why it fails. Nothing else is in the body, not even how long is left; a
/// guesser already knows the link has a password, and the sign-in page says the
/// same of a locked account. The 404 of an unknown, expired or switched-off link
/// is unchanged (A5).
fn locked() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        ApiJson(json!({ "error": "locked" })),
    )
        .into_response()
}

fn rejection(err: impl Into<ApiError>) -> Response {
    ApiRejection(err.into()).into_response()
}

/// `POST /api/public/dashboard/{token}/unlock` with `{password}`: the right
/// password gives a ticket (12 hours, or until the link expires) that the page
/// keeps for the browser session and sends as `X-Public-Link-Ticket`.
///
/// Public: the token and the password are the credentials. A locked link
/// answers `401 locked` and never reaches the hasher.
///
/// # Errors
///
/// `404 not_found` for a link that does not work; `400 no_password` for a link
/// without one; `401 wrong_password`, or `401 locked` once five tries are used; `400` for a body that is not
/// `{password}`; `503` without a database.
pub async fn unlock(
    State(state): State<AppState>,
    Path(token): Path<String>,
    body: Bytes,
) -> Response {
    let open = match resolve(&state, &token).await {
        Ok(Resolved::Open(open)) => open,
        Ok(Resolved::Gone) => return not_found(),
        Err(err) => return rejection(err),
    };
    let Some(hash) = open.link.password_hash.clone() else {
        return (
            StatusCode::BAD_REQUEST,
            ApiJson(json!({ "error": "no_password" })),
        )
            .into_response();
    };
    let Ok(parsed) = serde_json::from_slice::<UnlockBody>(&body) else {
        return rejection(ApiError::BadRequest(
            "body must be {\"password\": string}".to_owned(),
        ));
    };
    let pool = match pool_of(&state) {
        Ok(pool) => pool,
        Err(err) => return rejection(err),
    };
    let key = lock_key(&open.link.board_id);
    // BI-19 part A review fix (BLOCKER) R1: the try is counted BEFORE it is
    // checked, in one atomic statement. Counting after the hash let any number
    // of simultaneous guesses pass the lock check together, and each cost a
    // full Argon2 run. Only the first `LOCK_FAILURES` tries of a window reach
    // the verifier; the rest answer `locked` without hashing.
    let attempt = match throttle::record_attempt(pool, &key, &lock_policy()).await {
        Ok((n, _locked_until)) => n,
        Err(err) => return rejection(err),
    };
    if attempt > LOCK_FAILURES {
        return locked();
    }
    let password = Secret::new(parsed.password);
    let verified = tokio::task::spawn_blocking(move || {
        lakehouse_auth::password::verify_hash(&password, &hash)
    })
    .await
    .unwrap_or(false);
    if !verified {
        if attempt == LOCK_FAILURES {
            // The try that reached the limit set the lock. Best effort, like the
            // sign-in lock's record: the visitor's answer is the same either way.
            if let Err(err) = record_lock(pool, &open.link.board_id).await {
                tracing::warn!(%err, "failed to record dashboard.public_link_locked");
            }
            // The fifth wrong try is the one that locks the link: say so now.
            return locked();
        }
        return wrong_password();
    }
    if let Err(err) = throttle::clear(pool, &key).await {
        return rejection(err);
    }
    let ticket = lakehouse_auth::generate_opaque_token();
    let issued = public_links::issue_ticket(
        pool,
        &open.link.board_id,
        open.link.password_version,
        &lakehouse_auth::hash_token(&ticket),
    )
    .await;
    match issued {
        Ok(Some(expires_at)) => (
            StatusCode::OK,
            ApiJson(json!({ "ticket": ticket.expose(), "expiresAt": rfc3339(expires_at) })),
        )
            .into_response(),
        // The password changed, or the link ended, between the two statements:
        // the visitor typed a password that is no longer the password.
        Ok(None) => wrong_password(),
        Err(err) => rejection(store_failure(&err)),
    }
}

async fn record_lock(pool: &PgPool, board_id: &str) -> Result<(), lakehouse_store::StoreError> {
    let event = lakehouse_store::audit::NewAuditEvent {
        action: "dashboard.public_link_locked".to_owned(),
        outcome: "refused".to_owned(),
        resource_kind: Some("dashboard".to_owned()),
        resource_id: Some(board_id.to_owned()),
        ..Default::default()
    };
    lakehouse_store::audit::insert(pool, event)
        .await
        .map(|_| ())
}

// ── Managing a link (D5) ─────────────────────────────────────────────────

/// `?board=` of the two routes that take no body.
#[derive(Debug, Deserialize)]
pub struct BoardQuery {
    #[serde(default)]
    board: Option<String>,
}

/// `Some(None)` for a JSON `null`, `Some(Some(x))` for a value and `None` when
/// the key is absent (with `#[serde(default)]`), so "remove" and "keep" differ.
#[allow(
    clippy::option_option,
    reason = "absent, null and a value mean keep, remove and set; a three-case enum would need its own deserializer for the same three cases"
)]
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PutBody {
    board: String,
    expires_at: String,
    /// Absent: keep the password. `null`: remove it. A string: set it.
    #[allow(
        clippy::option_option,
        reason = "absent, null and a value mean keep, remove and set; a three-case enum would need its own deserializer for the same three cases"
    )]
    #[serde(default, deserialize_with = "present")]
    password: Option<Option<String>>,
    /// Make a new link (a new token) even though the dashboard has a live one.
    #[serde(default)]
    new_link: bool,
}

fn rfc3339(at: OffsetDateTime) -> String {
    at.format(&Rfc3339).unwrap_or_default()
}

/// A link as the Share dialog reads it. Never the password or its hash.
fn link_json(link: &PublicLink) -> Value {
    json!({
        "token": link.token,
        "expiresAt": rfc3339(link.expires_at),
        "expired": link.expires_at <= OffsetDateTime::now_utc(),
        "hasPassword": link.has_password(),
        "createdAt": rfc3339(link.created_at),
    })
}

/// The dashboard a management call is about: named, not the built-in one, and
/// visible to the caller (an unknown, trashed or somebody else's personal board
/// is the same "not found").
async fn managed_board(
    state: &AppState,
    principal: &Principal,
    id: Option<&str>,
) -> Result<Board, ApiError> {
    let id = id.map(str::trim).unwrap_or_default();
    if id.is_empty() || id == store::DEFAULT_BOARD_ID {
        return Err(ApiError::BadRequest("invalid dashboard".to_owned()));
    }
    store::get_board(&state.clickhouse, id, &viewer_of(principal))
        .await
        .map_err(|err| {
            upstream_error::ch_error_as(
                &upstream_error::DATABASE,
                &err,
                upstream_error::FailedAs::Internal,
            )
        })?
        .ok_or_else(|| ApiError::NotFound(BOARD_NOT_FOUND.to_owned()))
}

/// `GET /api/dashboard/public-link?board=` — the dashboard's link (or `null`),
/// and whether an administrator has switched public links off.
///
/// # Errors
///
/// `400` without a usable board id, `404` for a dashboard the caller cannot see,
/// `503` without a database.
pub async fn get_link(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Query(q): Query<BoardQuery>,
) -> ApiResult<ApiJson<Value>> {
    let board = managed_board(&state, &principal, q.board.as_deref()).await?;
    let pool = pool_of(&state)?;
    let enabled = crate::routes::settings::format_settings(&state)
        .await?
        .public_links_enabled;
    let link = public_links::get_for_board(pool, &board.id)
        .await
        .map_err(|err| store_failure(&err))?;
    Ok(ApiJson(json!({
        "enabled": enabled,
        "link": link.as_ref().map(link_json),
    })))
}

/// The expiry as a checked time.
fn checked_expiry(text: &str) -> Result<OffsetDateTime, ApiError> {
    let at = OffsetDateTime::parse(text.trim(), &Rfc3339)
        .map_err(|_| ApiError::BadRequest(EXPIRY_REFUSED.to_owned()))?;
    let now = OffsetDateTime::now_utc();
    if at < now + MIN_EXPIRY || at > now + MAX_EXPIRY {
        return Err(ApiError::BadRequest(EXPIRY_REFUSED.to_owned()));
    }
    Ok(at)
}

/// The `Argon2id` hash of a checked password, off the async threads (the hasher
/// takes tens of milliseconds of CPU).
async fn hashed(password: String) -> Result<String, ApiError> {
    let length = password.chars().count();
    if !(MIN_PASSWORD_CHARS..=MAX_PASSWORD_CHARS).contains(&length) {
        return Err(ApiError::BadRequest(PASSWORD_REFUSED.to_owned()));
    }
    tokio::task::spawn_blocking(move || {
        lakehouse_auth::password::hash_password(&Secret::new(password))
    })
    .await
    .map_err(|_| ApiError::Internal("authentication error".to_owned()))?
    .map(|hash| hash.expose().to_owned())
    .map_err(ApiError::from)
}

/// The audit entry of a change, written before the change: a link that cannot
/// be recorded is not made, changed or ended (`BI-1` D8, plan D5). Never the
/// token or the password.
async fn audit(
    state: &AppState,
    principal: &Principal,
    action: &str,
    board_id: &str,
    args: Value,
) -> Result<(), ApiError> {
    crate::routes::dashboard::record_download_audit(
        state,
        principal,
        (action, "dashboard", board_id),
        args,
    )
    .await
    .map_err(|_| ApiError::Unavailable(AUDIT_DOWN.to_owned()))
}

/// `PUT /api/dashboard/public-link` with `{board, expiresAt, password?,
/// newLink?}` — make the dashboard's link, or change the one it has.
///
/// With no live link, or `newLink: true`, a new link replaces the old one (a
/// new token; feature A6) and starts without a password unless one is sent.
/// Otherwise the live link keeps its token and takes the new expiry; the
/// password is kept when `password` is absent, removed when it is `null` and
/// replaced when it is a string (which ends every ticket issued before).
///
/// # Errors
///
/// `400` for a body of another shape, an expiry outside 1 hour to 365 days, a
/// password outside 8 to 128 characters, the built-in dashboard, a dashboard in
/// the personal space, or when an administrator has switched public links off;
/// `404` for a dashboard the caller cannot see; `503` without a database or when
/// the change cannot be audited.
pub async fn put_link(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let parsed: PutBody =
        serde_json::from_slice(&body).map_err(|_| ApiError::BadRequest(BODY_SHAPE.to_owned()))?;
    let board = managed_board(&state, &principal, Some(&parsed.board)).await?;
    if board.owner_id.is_some() {
        return Err(ApiError::BadRequest(store::PERSONAL_NOT_SHAREABLE.to_owned()).into());
    }
    let pool = pool_of(&state)?;
    if !crate::routes::settings::format_settings(&state)
        .await?
        .public_links_enabled
    {
        return Err(ApiError::BadRequest(SWITCHED_OFF.to_owned()).into());
    }
    let expires_at = checked_expiry(&parsed.expires_at)?;
    let new_hash = match &parsed.password {
        Some(Some(password)) => Some(hashed(password.clone()).await?),
        _ => None,
    };
    let existing = public_links::get_for_board(pool, &board.id)
        .await
        .map_err(|err| store_failure(&err))?;
    let live = existing
        .as_ref()
        .is_some_and(|l| l.expires_at > OffsetDateTime::now_utc());
    let replace = parsed.new_link || !live;
    let will_have_password = match (&parsed.password, replace) {
        (Some(Some(_)), _) => true,
        (Some(None), _) | (None, true) => false,
        (None, false) => existing.as_ref().is_some_and(PublicLink::has_password),
    };
    audit(
        &state,
        &principal,
        if replace {
            "dashboard.public_link_create"
        } else {
            "dashboard.public_link_update"
        },
        &board.id,
        json!({ "expiresAt": rfc3339(expires_at), "hasPassword": will_have_password }),
    )
    .await?;
    let created_by = match principal.id {
        PrincipalId::User(id) => Some(id),
        // `created_by` points at a person; a service identity is not one.
        PrincipalId::Service(_) => None,
    };
    let saved = if replace {
        // A new link starts from a clean count of wrong tries.
        throttle::clear(pool, &lock_key(&board.id)).await?;
        Some(
            public_links::replace(
                pool,
                &board.id,
                &store::new_public_token(),
                expires_at,
                new_hash.as_deref(),
                created_by,
            )
            .await
            .map_err(|err| store_failure(&err))?,
        )
    } else {
        let change = match (&parsed.password, new_hash) {
            (Some(Some(_)), Some(hash)) => PasswordChange::Set(hash),
            (Some(None), _) => PasswordChange::Remove,
            _ => PasswordChange::Keep,
        };
        let updated = public_links::update(pool, &board.id, expires_at, &change)
            .await
            .map_err(|err| store_failure(&err))?;
        if updated.is_some() && parsed.password.is_some() {
            // A new or removed password restarts the count of wrong tries.
            throttle::clear(pool, &lock_key(&board.id)).await?;
        }
        updated
    };
    // `None` only when the link expired between the read and the write; the
    // caller makes a new one.
    let Some(link) = saved else {
        return Err(
            ApiError::Conflict("The link has just expired. Make a new one.".to_owned()).into(),
        );
    };
    Ok(ApiJson(json!({ "ok": true, "link": link_json(&link) })))
}

/// `DELETE /api/dashboard/public-link?board=` — end the dashboard's link at
/// once. Allowed while an administrator's switch is off: ending narrows access.
///
/// # Errors
///
/// `400` without a usable board id, `404` for a dashboard the caller cannot see,
/// `503` without a database or when the change cannot be audited.
pub async fn delete_link(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Query(q): Query<BoardQuery>,
) -> ApiResult<ApiJson<Value>> {
    let board = managed_board(&state, &principal, q.board.as_deref()).await?;
    let pool = pool_of(&state)?;
    let had = public_links::get_for_board(pool, &board.id)
        .await
        .map_err(|err| store_failure(&err))?
        .is_some();
    if !had {
        return Ok(ApiJson(json!({ "ok": true, "ended": false })));
    }
    audit(
        &state,
        &principal,
        "dashboard.public_link_delete",
        &board.id,
        json!({}),
    )
    .await?;
    let ended = public_links::delete(pool, &board.id)
        .await
        .map_err(|err| store_failure(&err))?;
    Ok(ApiJson(json!({ "ok": true, "ended": ended })))
}

/// The live tokens of every dashboard, for the dashboard list's "public" mark.
/// A read that fails leaves the list without marks (and logs it under a
/// reference) rather than taking the whole list down.
pub(crate) async fn live_tokens(state: &AppState) -> std::collections::HashMap<String, String> {
    let Some(pool) = state.pg.as_deref() else {
        return std::collections::HashMap::new();
    };
    match public_links::live_tokens(pool).await {
        Ok(rows) => rows.into_iter().collect(),
        Err(err) => {
            let _logged = upstream_error::store(&upstream_error::STORE, &err);
            std::collections::HashMap::new()
        }
    }
}

/// Whether a link, as stored, still counts as one: it has not expired. An
/// expired link opens nothing and cannot be revived (an update refuses it), so
/// it must not stop its dashboard moving into the personal space.
fn counts_as_link(link: Option<&PublicLink>, now: OffsetDateTime) -> bool {
    link.is_some_and(|l| l.expires_at > now)
}

/// Whether a dashboard has a live public link, for the one rule that cares: it
/// cannot move into the personal space (`store::move_board`).
///
/// BI-19 part A review fix (SHOULD-FIX) R3: only a link that has not expired
/// counts.
///
/// # Errors
///
/// The classified store error.
pub(crate) async fn has_link(state: &AppState, board_id: &str) -> Result<bool, ApiError> {
    let Some(pool) = state.pg.as_deref() else {
        return Ok(false);
    };
    public_links::get_for_board(pool, board_id)
        .await
        .map(|l| counts_as_link(l.as_ref(), OffsetDateTime::now_utc()))
        .map_err(|err| store_failure(&err))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn an_expiry_must_be_a_time_between_an_hour_and_a_year_away() {
        let now = OffsetDateTime::now_utc();
        let at = |d: Duration| rfc3339(now + d);
        assert!(checked_expiry(&at(Duration::days(30))).is_ok());
        assert!(checked_expiry(&at(Duration::hours(2))).is_ok());
        assert!(checked_expiry(&at(Duration::days(364))).is_ok());
        for bad in [
            at(Duration::minutes(30)),
            at(Duration::days(366)),
            at(Duration::days(-1)),
            "tomorrow".to_owned(),
            String::new(),
            "2026-13-45T00:00:00Z".to_owned(),
        ] {
            assert!(
                matches!(checked_expiry(&bad), Err(ApiError::BadRequest(m)) if m == EXPIRY_REFUSED),
                "{bad}"
            );
        }
    }

    #[test]
    fn the_lock_key_follows_the_dashboard_and_is_not_the_login_key_of_anyone() {
        assert_eq!(lock_key("b_1a2b"), lock_key("b_1a2b"));
        assert_ne!(lock_key("b_1a2b"), lock_key("b_1a2c"));
        assert_ne!(lock_key("b_1a2b"), throttle::key_for("b_1a2b"));
        // BI-19 part A review fix (SHOULD-FIX) R2: no login can produce it. A
        // login key is the 64 hex characters `key_for` makes of an email.
        assert_ne!(lock_key("b_1a2b"), throttle::key_for("public-link:b_1a2b"));
        let login_keys_are_hex =
            |k: &str| k.len() == 64 && k.chars().all(|c| c.is_ascii_hexdigit());
        assert!(!login_keys_are_hex(&lock_key("b_1a2b")));
    }

    /// BI-19 part A review fix (SHOULD-FIX) R3.
    #[test]
    fn only_a_link_that_has_not_expired_counts_as_a_link() {
        let now = OffsetDateTime::now_utc();
        let link = |expires_at| PublicLink {
            board_id: "b_1".to_owned(),
            token: "p_token".to_owned(),
            expires_at,
            password_hash: None,
            password_version: 0,
            created_by: None,
            created_at: now,
            updated_at: now,
        };
        assert!(counts_as_link(Some(&link(now + Duration::days(1))), now));
        assert!(!counts_as_link(
            Some(&link(now - Duration::seconds(1))),
            now
        ));
        assert!(!counts_as_link(None, now));
    }

    #[test]
    fn the_lock_is_five_in_fifteen_minutes_for_fifteen() {
        let policy = lock_policy();
        assert_eq!(policy.max_failures, 5);
        assert_eq!(policy.window, Duration::minutes(15));
        assert_eq!(policy.lockout, Duration::minutes(15));
    }

    #[test]
    fn a_body_tells_a_missing_password_from_a_null_one() {
        let parse = |text: &str| serde_json::from_str::<PutBody>(text).unwrap().password;
        let base = r#""board":"b_1","expiresAt":"2030-01-01T00:00:00Z""#;
        assert_eq!(parse(&format!("{{{base}}}")), None);
        assert_eq!(parse(&format!("{{{base},\"password\":null}}")), Some(None));
        assert_eq!(
            parse(&format!("{{{base},\"password\":\"secret-pw\"}}")),
            Some(Some("secret-pw".to_owned()))
        );
        assert!(
            serde_json::from_str::<PutBody>(&format!("{{{base},\"token\":\"x\"}}")).is_err(),
            "an unknown field is refused"
        );
    }

    #[test]
    fn the_link_json_never_carries_the_password_or_its_hash() {
        let link = PublicLink {
            board_id: "b_1".to_owned(),
            token: "p_token".to_owned(),
            expires_at: OffsetDateTime::now_utc() + Duration::days(1),
            password_hash: Some("$argon2id$v=19$hash".to_owned()),
            password_version: 3,
            created_by: None,
            created_at: OffsetDateTime::now_utc(),
            updated_at: OffsetDateTime::now_utc(),
        };
        let text = link_json(&link).to_string();
        assert!(!text.contains("argon2"));
        assert!(!text.contains("password_hash"));
        assert!(text.contains("\"hasPassword\":true"));
        assert!(text.contains("\"expired\":false"));
    }
}
