//! The door: outside tools read published Gold tables through the API, and
//! every read is counted (DATA-5, ADR 0016).
//!
//! # What this is
//!
//! ADR 0016 makes the API the only door for outside readers. This module
//! serves the read side of the Iceberg REST catalog under `/api/iceberg/v1`
//! and forwards each allowed call to `Lakekeeper` under ONE catalog
//! principal of its own (`outside-readers`) — the outside tool's own key
//! never reaches `Lakekeeper` — and counts a successful table load.
//!
//! # The rules, and where each one lives
//!
//! | Rule | Where |
//! | --- | --- |
//! | Only a service identity may use the door (D1) | [`refuse`] |
//! | Only the `gold` namespace, and only reads (D2) | [`namespace_is_gold`], [`method_is_served`] |
//! | A table name this door may forward (DATA-5 S1 review BLOCKER 1) | [`table_name_is_allowed`] |
//! | Forwarded under `outside-readers`, never the tool's key (D3) | [`call_catalog`] |
//! | The published storage address, or an honest "not set up" (D4) | [`setup`], [`rewrite_endpoint`] |
//! | A load that cannot be counted is refused (D6) | [`record_read`] |
//! | The catalog tenant's warehouse only (D8) | [`expected_prefix`] |
//!
//! # Why the door answers in the Iceberg error shape
//!
//! The caller is an Iceberg client, not the console: it expects
//! `{"error": {"message", "type", "code"}}` (the Iceberg REST spec's shape)
//! and reads the status. The console's own `{"error": "<text>"}` shape
//! would be a second, differently-shaped error in the same API, so every
//! refusal THIS module makes about the REQUEST uses the Iceberg shape.
//! The two answers that are about the DEPLOYMENT rather than the request
//! (outside reading not set up, the catalog unreachable) are `ApiError`s,
//! because they are this service's own answers and not the catalog's.
//!
//! # The order of the checks, and why
//!
//! Every route answers in this order: (1) is outside reading set up on
//! this deployment at all, (2) is the caller a service identity, (3) is
//! the method one this route serves, (4) is the namespace `gold`, (5) is
//! the table name one this door may forward, (6) is the warehouse this
//! deployment's. The deployment's own state comes first because a
//! deployment that has not published its storage address has no door to
//! answer with — every request, from every caller, gets the same 503
//! naming the setting that is missing (D4).
//!
//! # How the forwarded path is built
//!
//! The path is built one segment at a time — the `url` crate's
//! `path_segments_mut().push` — never by pasting a caller's value into a
//! string. `push` percent-encodes `/` and `%`, so a segment can never
//! become a separator or an escape, whatever it holds (DATA-5 S1 review
//! BLOCKER 1). The caller's own table name is checked before anything is
//! forwarded as well — see [`table_name_is_allowed`].
//!
//! # Two rewrites the door makes on the way out
//!
//! 1. **The storage address** (ADR 0016 point 4). `Lakekeeper` vends the
//!    deployment's own internal name (`http://rustfs:9000`), which
//!    resolves only inside the network, so the door swaps it for
//!    `OUTSIDE_S3_ENDPOINT` — the address an operator publishes. See
//!    [`rewrite_endpoint`].
//! 2. **The catalog's own `uri` override** (in [`strip_uri_override`]).
//!    `Lakekeeper`'s config answers with `overrides.uri` naming ITSELF.
//!    An Iceberg client that follows that override — `pyiceberg` does, it
//!    sets its base URI from the config — would leave the door and talk to
//!    the catalog directly, and that read would be invisible to the
//!    count. The door therefore removes the override, which leaves the
//!    client on the address it was configured with: the door's own. This
//!    is the same reason the tool's key never reaches `Lakekeeper`
//!    (ADR 0016 point 3) — a read that skips the door is a read nobody
//!    counts.
//!
//! # Cost, stated plainly
//!
//! Every call that names a warehouse prefix is checked against the
//! prefix `Lakekeeper`'s own config advertises, which costs one extra
//! `GET /v1/config` per call. That is deliberate: the prefix is the
//! warehouse's own id (not the `LAKEKEEPER_WAREHOUSE` name), it is only
//! knowable from the catalog, and a cached copy could outlive the
//! warehouse it named. The door is an outside-tool surface, not a hot
//! path, and the config call is a few hundred bytes.
//!
//! # What this module deliberately does NOT do
//!
//! It does not count a read that skips the door (internal storage keys,
//! or `Lakekeeper`'s own published port — ADR 0016's "Consequences"), it
//! does not serve a tenant warehouse (D8), and it does not offer the
//! `OAuth2` client-credentials exchange (the spec's "Not included").

use std::sync::OnceLock;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, Method, StatusCode};
use lakehouse_auth::{Principal, PrincipalId};
use lakehouse_core::ApiError;
use lakehouse_core::secret::SecretValue;
use serde_json::{Value, json};

use crate::auth::AuthenticatedPrincipal;
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::lakekeeper_token::{read_token_file, token_unavailable_message};
use crate::state::AppState;
use crate::upstream_error::{self, Class, FailedAs};

/// The one namespace the door serves (ADR 0016 point 2).
pub const GOLD_NAMESPACE: &str = "gold";

/// The catalog principal the door forwards an outside tool's call under
/// (ADR 0016 point 3).
///
/// The tool's own key never reaches the catalog. This is the principal
/// `ops/oidc-mock` mints a token for, `docker-compose.yml`'s
/// `lakekeeper-authz-init` registers and grants `describe` on, and the
/// Gold export grants `select` on the `gold` namespace to
/// (`crate::gold_export::grant_outside_readers_select`).
pub const OUTSIDE_READERS_PRINCIPAL: &str = "outside-readers";

/// The refusal every "that is not yours to read" answer carries
/// (ADR 0016 point 2).
const REFUSED_MESSAGE: &str = "Outside readers may only read published Gold tables.";

/// The refusal for a caller that is not a service identity (ADR 0016
/// point 1, plan D1). A browser session is refused too: the count is per
/// tool, and a session is not a tool.
const NOT_A_SERVICE_MESSAGE: &str = "Use a service identity's key: the door counts reads per tool.";

/// The refusal for a warehouse prefix that is not this deployment's
/// warehouse (D2, D8).
const WRONG_WAREHOUSE_MESSAGE: &str = "No such warehouse for this catalog.";

/// The purpose label `read_token_file` names in its fixed 503 text.
const OUTSIDE_READ_PURPOSE: &str = "outside-readers";

/// The env var `read_token_file` names in its fixed 503 text.
const OUTSIDE_READ_TOKEN_ENV_VAR: &str = "LAKEKEEPER_OUTSIDE_READ_TOKEN_FILE";

/// The env var that publishes the storage address an outside tool can
/// reach. Unset (or empty) means outside reading is OFF.
const OUTSIDE_S3_ENDPOINT_ENV_VAR: &str = "OUTSIDE_S3_ENDPOINT";

/// The header that asks `Lakekeeper` to vend short-lived storage
/// credentials — the same header `lakehouse-iceberg` sets on every call
/// (see that crate's module doc comment: no code path in it is correct
/// without vended credentials).
const DELEGATION_HEADER: &str = "x-iceberg-access-delegation";

/// The value of [`DELEGATION_HEADER`] that asks for vended (not remotely
/// signed) credentials.
const VENDED_CREDENTIALS: &str = "vended-credentials";

/// How long a forwarded call to `Lakekeeper` may take.
const FORWARD_TIMEOUT: Duration = Duration::from_secs(30);

/// The day (as a Julian day number) this process last pruned the read
/// history. ADR 0016 point 5: "Rows older than 90 days are deleted" — and
/// the prune runs at most once a day per process, not per read.
static LAST_PRUNE_DAY: AtomicI64 = AtomicI64::new(i64::MIN);

/// The door's response: a status and an Iceberg-shaped JSON body.
///
/// A tuple rather than a bare [`ApiJson`], because the door forwards
/// `Lakekeeper`'s own status and the Iceberg error shape carries its own
/// code — the same `(StatusCode, ApiJson<T>)` shape
/// [`crate::error::ApiRejection`] renders.
type DoorResponse = (StatusCode, ApiJson<Value>);

/// The Iceberg REST error shape: `{"error": {"message", "type", "code"}}`.
///
/// `code` is the HTTP status the answer carries, so a client that only
/// reads the body still learns what happened.
fn iceberg_error(message: &str, kind: &str, code: u16) -> Value {
    json!({ "error": { "message": message, "type": kind, "code": code } })
}

/// The refusal the door makes about WHAT may be read.
fn refused() -> DoorResponse {
    (
        StatusCode::FORBIDDEN,
        ApiJson(iceberg_error(REFUSED_MESSAGE, "ForbiddenException", 403)),
    )
}

/// The refusal for a caller that is not a service identity.
fn not_a_service() -> DoorResponse {
    (
        StatusCode::FORBIDDEN,
        ApiJson(iceberg_error(
            NOT_A_SERVICE_MESSAGE,
            "ForbiddenException",
            403,
        )),
    )
}

/// The refusal for a warehouse prefix that is not this deployment's.
fn wrong_warehouse() -> DoorResponse {
    (
        StatusCode::NOT_FOUND,
        ApiJson(iceberg_error(
            WRONG_WAREHOUSE_MESSAGE,
            "NoSuchWarehouseException",
            404,
        )),
    )
}

/// The 503 for a deployment where outside reading is not set up
/// (ADR 0016 point 4, plan D4).
fn not_set_up(problem: NotSetUp) -> ApiError {
    ApiError::Unavailable(problem.reason())
}

/// Why outside reading is not set up on this deployment (ADR 0016 point
/// 4, plan D4).
///
/// A small type rather than an [`ApiError`], because the console's
/// consumers route (`routes::gold::consumers`) asks the same question
/// (plan D7) and must answer with the SAME sentence the door's own 503
/// carries. The sentence is therefore built here, once, from this value
/// — and no route ever puts an error's own text into a response
/// (SEC-11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NotSetUp {
    /// The published storage address (`OUTSIDE_S3_ENDPOINT`) is unset:
    /// an outside tool cannot reach the storage the catalog vends.
    Endpoint,
    /// The `outside-readers` token file is unset: the door has no
    /// catalog principal to forward under.
    TokenFile,
    /// The `outside-readers` token file is set but cannot be read: the
    /// door has no credential to forward with.
    TokenUnreadable,
}

impl NotSetUp {
    /// The sentence that says outside reading is not set up, naming what
    /// is missing (ADR 0016 point 4, plan D4/D7).
    ///
    /// The door's 503 and the console's "Not measured" carry this same
    /// sentence: one sentence, two callers.
    pub(crate) fn reason(self) -> String {
        match self {
            Self::Endpoint => {
                format!("Outside reading is not set up: {OUTSIDE_S3_ENDPOINT_ENV_VAR} is not set.")
            }
            Self::TokenFile => {
                format!("Outside reading is not set up: {OUTSIDE_READ_TOKEN_ENV_VAR} is not set.")
            }
            Self::TokenUnreadable => {
                token_unavailable_message(OUTSIDE_READ_PURPOSE, OUTSIDE_READ_TOKEN_ENV_VAR)
            }
        }
    }
}

/// The outside-reading settings this deployment has.
pub(crate) struct Setup {
    /// The storage address an operator published (`OUTSIDE_S3_ENDPOINT`).
    endpoint: String,
    /// The `outside-readers` principal's bearer token — the identity the
    /// door forwards under, never the tool's own key.
    token: SecretValue,
}

/// Read the outside-reading settings, or say which one is missing (D4).
/// The check runs before anything is forwarded: a deployment that has
/// not published its storage address must not answer a read at all.
///
/// `pub(crate)`, because `routes::gold::consumers` asks the same
/// question — "is outside reading set up on this deployment?" — and must
/// answer with the SAME sentence the door's own 503 carries (D7). One
/// check, one sentence, two callers.
///
/// # Errors
///
/// Returns the [`NotSetUp`] that says what is missing: the published
/// storage address, the token file, or the token file's unreadability.
pub(crate) async fn setup(state: &AppState) -> Result<Setup, NotSetUp> {
    let Some(endpoint) = state.config.outside_s3_endpoint.clone() else {
        return Err(NotSetUp::Endpoint);
    };
    let token_path = state.config.lakekeeper_outside_read_token_file.trim();
    if token_path.is_empty() {
        return Err(NotSetUp::TokenFile);
    }
    // The helper logs the io error and answers with its own fixed
    // sentence; the reason it could not read the file is the same one
    // the door's 503 carries (ADR 0011, AGENTS.md rule 4).
    let token = read_token_file(token_path, OUTSIDE_READ_PURPOSE, OUTSIDE_READ_TOKEN_ENV_VAR)
        .await
        .map_err(|_| NotSetUp::TokenUnreadable)?;
    Ok(Setup { endpoint, token })
}

/// The refusal this request earns, if any: a caller that is not a
/// service identity, or a method this route does not serve.
///
/// Both are the same shape (the Iceberg error shape, 403): a write looks
/// like every other refusal the door makes, and a session looks like a
/// write — neither is a thing this door answers.
fn refuse(principal: &Principal, route: Route, method: &Method) -> Option<DoorResponse> {
    if !matches!(principal.id, PrincipalId::Service(_)) {
        return Some(not_a_service());
    }
    if !method_is_served(route, method) {
        return Some(refused());
    }
    None
}

/// Only the `gold` namespace is served (ADR 0016 point 2).
fn namespace_is_gold(namespace: &str) -> bool {
    namespace == GOLD_NAMESPACE
}

/// The longest table name the door forwards.
///
/// A name longer than this is not a table the Gold export wrote: the
/// export's own names are a mart name in lower case, digits and
/// underscores (`lakehouse_iceberg::gold::sanitize_mart_name`), so the
/// longest real name is far shorter than this.
const MAX_TABLE_NAME_LEN: usize = 255;

/// Whether `table` is a table name this door may forward.
///
/// DATA-5 S1 review BLOCKER 1: `axum` percent-decodes the `{table}` path
/// value, so `%2F..%2F..` arrives as `../..`. A name pasted into a
/// forwarded URL with `format!` then walks out of the `gold` namespace
/// (`.../namespaces/gold/tables/../../../silver/tables/x`), and the URL
/// parser resolves the `..` — the call leaves the namespace the door
/// promised, and a made-up table name would be counted as a read.
///
/// Only a name of ASCII letters, digits, `_` and `-` is accepted: no dot,
/// no slash, no percent, so nothing in it can be a path separator or an
/// escape. Every name the Gold export writes (`orders`,
/// `gold_export_smoke`) passes.
fn table_name_is_allowed(table: &str) -> bool {
    !table.is_empty()
        && table.len() <= MAX_TABLE_NAME_LEN
        && table
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// The HTTP client used to reach `Lakekeeper`.
///
/// A plain `reqwest` client, with redirects off and a bounded timeout.
/// The connector allowlist (`connector_probe::resolve_checked`) does NOT
/// apply here: `Lakekeeper` is this deployment's own configured host,
/// read from configuration and never from a request, so there is no
/// caller-controlled target to police — the same reasoning
/// `lakehouse_auth::oidc`'s JWKS fetch and `lakehouse_auth::openfga`'s
/// management client already follow.
fn client() -> Option<&'static reqwest::Client> {
    static CLIENT: OnceLock<Option<reqwest::Client>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                // No redirects: `Lakekeeper` is a configured internal host,
                // and a redirect could send the door (and the tool's read)
                // to a host the operator never configured.
                .redirect(reqwest::redirect::Policy::none())
                .timeout(FORWARD_TIMEOUT)
                .build()
                .map_or_else(
                    |err| {
                        // The builder above takes no request input, so a
                        // failure here is a broken TLS backend, not a
                        // request condition. The door then answers 503
                        // instead of panicking.
                        tracing::error!(%err, "the Lakekeeper HTTP client could not be built");
                        None
                    },
                    Some,
                )
        })
        .as_ref()
}

/// One forwarded call: the status `Lakekeeper` answered and its JSON body.
struct CatalogAnswer {
    status: StatusCode,
    body: Value,
}

/// The query the door adds to a forwarded call: the tool's own query,
/// unchanged, plus the warehouse this deployment serves.
///
/// `Lakekeeper`'s config endpoint needs the warehouse to resolve the
/// warehouse AT ALL (it answers 400 without it), which is why the door
/// sends it — `iceberg-catalog-rest` does the same for the API's own
/// catalog client (see that crate's `load_config`).
fn query_for(state: &AppState, raw: Option<&str>, with_warehouse: bool) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = raw
        .map(|raw| {
            // DATA-5 S1 review SHOULD-FIX 1: the pairs are DECODED here
            // and encoded once, by `RequestBuilder::query`. Splitting the
            // raw query without decoding it left the encoding to happen
            // twice, so a `pageToken=a%3D` reached the catalog as
            // `pageToken=a%253D` and paging through a long table list
            // broke. Decoding also means the `warehouse` this door
            // replaces below is recognised however the tool spelled it.
            url::form_urlencoded::parse(raw.as_bytes())
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect()
        })
        .unwrap_or_default();
    if with_warehouse {
        pairs.retain(|(key, _)| key != "warehouse");
        pairs.push((
            "warehouse".to_owned(),
            state.config.lakekeeper_warehouse.clone(),
        ));
    }
    pairs
}

/// The URL a forwarded call goes to: the catalog's own address with the
/// call's path segments appended, each one percent-encoded.
///
/// DATA-5 S1 review BLOCKER 1: the path is built segment by segment (the
/// `url` crate's `path_segments_mut().push`), never by pasting a value
/// into a string. `push` percent-encodes `/` and `%`, so a segment can
/// never become a separator or an escape, whatever it holds.
///
/// `push` DROPS a segment that is exactly `.` or `..` instead of
/// encoding it, so such a segment would change the path's shape rather
/// than stay inside it. The table name check
/// ([`table_name_is_allowed`]) refuses those long before this point; the
/// only other variable segment is the warehouse prefix the catalog itself
/// advertised, so this guard is about a misconfigured catalog.
///
/// # Errors
///
/// The configured catalog URI does not parse, or it cannot carry a path
/// (`path_segments_mut` refuses such a URL). Both mean this deployment
/// cannot reach the catalog at all, so the answer is the catalog's own
/// fixed "unavailable" text — never the URI, never the parser's text.
fn forwarded_url(base: &str, segments: &[&str]) -> Result<url::Url, ApiError> {
    let mut url = url::Url::parse(base.trim())
        .map_err(|_| ApiError::Unavailable(upstream_error::CATALOG.unavailable.to_owned()))?;
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|()| ApiError::Unavailable(upstream_error::CATALOG.unavailable.to_owned()))?;
        // The base address may end in a slash; without this the first
        // segment would land behind a doubled separator.
        path.pop_if_empty();
        for segment in segments {
            if matches!(*segment, "." | "..") {
                return Err(ApiError::Unavailable(
                    upstream_error::CATALOG.unavailable.to_owned(),
                ));
            }
            path.push(segment);
        }
    }
    Ok(url)
}

/// Forward one call to `Lakekeeper` and read its answer.
///
/// The tool's own credential is never forwarded: the request carries the
/// `outside-readers` token and nothing else of the caller's. The
/// `X-Iceberg-Access-Delegation` header is the tool's when it sent one
/// (the door does not second-guess a client that asks for a specific
/// delegation mode) and `vended-credentials` when it did not — the door
/// promises the tool short-lived, table-scoped keys (ADR 0016 point 4),
/// and a call without the header gets no keys at all.
///
/// # Errors
///
/// A transport failure or an unreadable body is reported through
/// [`upstream_error`], so the caller's answer carries a fixed message and
/// a reference id, never `Lakekeeper`'s own text (SEC-11).
async fn call_catalog(
    state: &AppState,
    method: &Method,
    segments: &[&str],
    query: &[(String, String)],
    setup: &Setup,
    headers: &HeaderMap,
) -> Result<CatalogAnswer, ApiError> {
    let Some(client) = client() else {
        return Err(ApiError::Unavailable(
            upstream_error::CATALOG.unavailable.to_owned(),
        ));
    };
    let url = forwarded_url(&state.config.lakekeeper_catalog_uri, segments)?;
    let delegation = headers
        .get(DELEGATION_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .unwrap_or(VENDED_CREDENTIALS);
    let mut request = client
        .request(method.clone(), url)
        .bearer_auth(setup.token.expose_secret())
        .header(DELEGATION_HEADER, delegation);
    if !query.is_empty() {
        request = request.query(query);
    }
    let response = request.send().await.map_err(|err| {
        upstream_error::report(&upstream_error::CATALOG, Class::Unavailable, &err)
            .into_api_error(FailedAs::ServiceUnavailable)
    })?;
    let status = response.status();
    // A HEAD answer has no body to read: the status IS the answer.
    if method == Method::HEAD {
        return Ok(CatalogAnswer {
            status,
            body: Value::Null,
        });
    }
    let bytes = response.bytes().await.map_err(|err| {
        upstream_error::report(&upstream_error::CATALOG, Class::Unavailable, &err)
            .into_api_error(FailedAs::ServiceUnavailable)
    })?;
    let body: Value = serde_json::from_slice(&bytes).map_err(|err| {
        upstream_error::report(&upstream_error::CATALOG, Class::Failed, &err).into_bad_gateway()
    })?;
    Ok(CatalogAnswer { status, body })
}

/// The answer the door gives for a call `Lakekeeper` refused.
///
/// The status comes back (`Lakekeeper`'s own), but the body does not:
/// it is replaced by the Iceberg error shape with fixed text, because
/// upstream error text never reaches a response (principle 4, SEC-11).
fn refused_by_catalog(status: StatusCode) -> DoorResponse {
    let kind = if status.is_client_error() {
        "BadRequestException"
    } else {
        "InternalServerError"
    };
    (
        status,
        ApiJson(iceberg_error(
            "The catalog refused this request.",
            kind,
            status.as_u16(),
        )),
    )
}

/// The warehouse prefix `Lakekeeper`'s own config advertises — the one
/// warehouse this door serves (D2, D8).
///
/// The prefix is the warehouse's id, not the `LAKEKEEPER_WAREHOUSE`
/// name, so it is only knowable by asking the catalog.
async fn expected_prefix(
    state: &AppState,
    setup: &Setup,
    headers: &HeaderMap,
) -> Result<String, ApiError> {
    let answer = call_catalog(
        state,
        &Method::GET,
        &["v1", "config"],
        &query_for(state, None, true),
        setup,
        headers,
    )
    .await?;
    if !answer.status.is_success() {
        return Err(ApiError::Unavailable(
            upstream_error::CATALOG.unavailable.to_owned(),
        ));
    }
    answer
        .body
        .get("defaults")
        .and_then(Value::as_object)
        .and_then(|defaults| defaults.get("prefix"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| ApiError::Unavailable(upstream_error::CATALOG.unavailable.to_owned()))
}

/// Replace the vended `s3.endpoint` with the published one
/// (ADR 0016 point 4, plan D4).
///
/// The vended endpoint is the deployment's internal name; an outside tool
/// cannot resolve it. The keys are NOT replaced: they are still the
/// catalog's short-lived, table-scoped ones.
///
/// Only a key that IS there is replaced: a profile that vends no
/// `s3.endpoint` at all has no address to swap, and inventing one would
/// be a lie about where the data lives.
fn rewrite_endpoint(body: &mut Value, endpoint: &str) {
    if let Some(config) = body.get_mut("config").and_then(Value::as_object_mut)
        && config.contains_key("s3.endpoint")
    {
        config.insert("s3.endpoint".to_owned(), Value::String(endpoint.to_owned()));
    }
    let Some(credentials) = body
        .get_mut("storage-credentials")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for credential in credentials {
        if let Some(config) = credential.get_mut("config").and_then(Value::as_object_mut)
            && config.contains_key("s3.endpoint")
        {
            config.insert("s3.endpoint".to_owned(), Value::String(endpoint.to_owned()));
        }
    }
}

/// Remove `Lakekeeper`'s own `uri` override from the config the door
/// forwards (see this module's doc comment, "Two rewrites").
///
/// A client that follows the override leaves the door, and a read that
/// leaves the door is a read nobody counts.
fn strip_uri_override(body: &mut Value) {
    for section in ["overrides", "defaults"] {
        if let Some(values) = body.get_mut(section).and_then(Value::as_object_mut) {
            values.remove("uri");
        }
    }
}

/// Keep only the `gold` namespace in a namespace list (ADR 0016 point 2).
///
/// A filter, not a fabrication: if `gold` does not exist yet, the list is
/// empty — the door never reports a namespace the catalog does not have.
fn filter_namespaces_to_gold(body: &mut Value) {
    let Some(namespaces) = body.get_mut("namespaces").and_then(Value::as_array_mut) else {
        return;
    };
    namespaces.retain(|namespace| {
        namespace
            .as_array()
            .is_some_and(|parts| parts.len() == 1 && parts[0].as_str() == Some(GOLD_NAMESPACE))
    });
}

/// The Postgres pool, or a 503 that says there is none (the same posture
/// `routes::identity::pool` takes).
fn pool(state: &AppState) -> Result<&lakehouse_store::PgPool, ApiError> {
    state.pg.as_deref().ok_or_else(|| {
        ApiError::Unavailable(
            "database error: no Postgres pool is configured (DATABASE_URL is missing or not a \
             valid Postgres connection string)"
                .to_owned(),
        )
    })
}

/// Count one read of `table` for `principal` (ADR 0016 point 5, plan D6).
///
/// A load whose count cannot be written is REFUSED, not served
/// uncounted: an uncounted read would make the count a lie (principle
/// 5). The message is the store's own fixed text; the driver's text goes
/// to the log, never to the caller (SEC-11).
async fn record_read(state: &AppState, principal: &Principal, table: &str) -> Result<(), ApiError> {
    let pool = pool(state)?;
    lakehouse_store::outside_reads::record_outside_read(
        pool,
        principal.id.uuid(),
        &principal.display_name,
        GOLD_NAMESPACE,
        table,
    )
    .await
    .map_err(|err| {
        tracing::error!(%err, "the outside read could not be counted; the load is refused");
        ApiError::Unavailable("database error".to_owned())
    })?;
    prune_once_per_day(state).await;
    Ok(())
}

/// Delete the read history older than the retention window, at most once
/// a day per process (ADR 0016 point 5).
///
/// A failure here is logged, not raised: the pruning is maintenance, and
/// the count that was just written is already stored.
async fn prune_once_per_day(state: &AppState) {
    let today = i64::from(time::OffsetDateTime::now_utc().date().to_julian_day());
    if LAST_PRUNE_DAY.load(Ordering::Acquire) == today {
        return;
    }
    let Some(pool) = state.pg.as_deref() else {
        return;
    };
    match lakehouse_store::outside_reads::prune_outside_reads(
        pool,
        lakehouse_store::outside_reads::RETENTION_DAYS,
    )
    .await
    {
        Ok(removed) => {
            LAST_PRUNE_DAY.store(today, Ordering::Release);
            if removed > 0 {
                tracing::info!(removed, "pruned outside-read history older than the window");
            }
        }
        Err(err) => {
            tracing::warn!(%err, "the outside-read history could not be pruned");
        }
    }
}

/// The door's routes.
///
/// Every route is registered for ANY method on purpose: the door answers
/// a refused method with the Iceberg error shape (403), not with axum's
/// 405, and a 405 is a shape the Iceberg clients this door exists for do
/// not read. The method each route actually SERVES is checked inside the
/// handler — see [`method_is_served`] and the `POLICY_TABLE` entries that
/// name it.
pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/api/iceberg/v1/config", axum::routing::any(config))
        .route(
            "/api/iceberg/v1/{prefix}/namespaces",
            axum::routing::any(namespaces),
        )
        .route(
            "/api/iceberg/v1/{prefix}/namespaces/{namespace}",
            axum::routing::any(namespace),
        )
        .route(
            "/api/iceberg/v1/{prefix}/namespaces/{namespace}/tables",
            axum::routing::any(tables),
        )
        .route(
            "/api/iceberg/v1/{prefix}/namespaces/{namespace}/tables/{table}",
            axum::routing::any(table),
        )
        .route(
            "/api/iceberg/v1/{prefix}/namespaces/{namespace}/tables/{table}/credentials",
            axum::routing::any(credentials),
        )
        .route("/api/iceberg/{*rest}", axum::routing::any(catch_all))
}

/// Whether `method` is one the door SERVES on this route.
///
/// The table below is the plan's route table (D2): the door reads the
/// catalog and nothing else. A method that is not listed is refused with
/// the Iceberg error shape — the same shape the catch-all uses, so a
/// write looks like every other refusal the door makes.
fn method_is_served(route: Route, method: &Method) -> bool {
    match route {
        // A HEAD is served on the namespace and on the table: the "does
        // it exist" probe (D2's table). The table's HEAD is NOT counted;
        // its GET is — see `table`'s handler.
        Route::Namespace | Route::Table => method == Method::GET || method == Method::HEAD,
        // Everything else is a plain GET: the door reads the catalog and
        // nothing else.
        Route::Config | Route::Namespaces | Route::Tables | Route::Credentials => {
            method == Method::GET
        }
    }
}

/// The door's routes, as the ones the method table above names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    Config,
    Namespaces,
    Namespace,
    Tables,
    Table,
    Credentials,
}

/// `GET /api/iceberg/v1/config` — the catalog's own config, so a client
/// learns the prefix it must use (D2).
///
/// The `uri` override is removed (see [`strip_uri_override`]); the
/// `defaults.prefix` is NOT, because that is the warehouse this door
/// serves and the client needs it.
pub async fn config(
    State(state): State<AppState>,
    AuthenticatedPrincipal(principal): AuthenticatedPrincipal,
    method: Method,
    headers: HeaderMap,
    query: RawQuery,
) -> ApiResult<DoorResponse> {
    let setup = setup(&state).await.map_err(not_set_up)?;
    if let Some(response) = refuse(&principal, Route::Config, &method) {
        return Ok(response);
    }
    let answer = call_catalog(
        &state,
        &Method::GET,
        &["v1", "config"],
        &query_for(&state, query.0.as_deref(), true),
        &setup,
        &headers,
    )
    .await?;
    if !answer.status.is_success() {
        return Ok(refused_by_catalog(answer.status));
    }
    let mut body = answer.body;
    strip_uri_override(&mut body);
    Ok((answer.status, ApiJson(body)))
}

/// `GET /api/iceberg/v1/{prefix}/namespaces` — the namespaces, filtered
/// to `gold` (D2).
pub async fn namespaces(
    State(state): State<AppState>,
    AuthenticatedPrincipal(principal): AuthenticatedPrincipal,
    Path(prefix): Path<String>,
    method: Method,
    headers: HeaderMap,
    query: RawQuery,
) -> ApiResult<DoorResponse> {
    let setup = setup(&state).await.map_err(not_set_up)?;
    if let Some(response) = refuse(&principal, Route::Namespaces, &method) {
        return Ok(response);
    }
    let expected = expected_prefix(&state, &setup, &headers).await?;
    if prefix != expected {
        return Ok(wrong_warehouse());
    }
    let answer = call_catalog(
        &state,
        &Method::GET,
        &["v1", prefix.as_str(), "namespaces"],
        &query_for(&state, query.0.as_deref(), false),
        &setup,
        &headers,
    )
    .await?;
    if !answer.status.is_success() {
        return Ok(refused_by_catalog(answer.status));
    }
    let mut body = answer.body;
    filter_namespaces_to_gold(&mut body);
    Ok((answer.status, ApiJson(body)))
}

/// `GET`/`HEAD /api/iceberg/v1/{prefix}/namespaces/{namespace}` — one
/// namespace's metadata (D2). Only `gold` is served.
pub async fn namespace(
    State(state): State<AppState>,
    AuthenticatedPrincipal(principal): AuthenticatedPrincipal,
    Path((prefix, namespace)): Path<(String, String)>,
    method: Method,
    headers: HeaderMap,
    query: RawQuery,
) -> ApiResult<DoorResponse> {
    let setup = setup(&state).await.map_err(not_set_up)?;
    if let Some(response) = refuse(&principal, Route::Namespace, &method) {
        return Ok(response);
    }
    if !namespace_is_gold(&namespace) {
        return Ok(refused());
    }
    let expected = expected_prefix(&state, &setup, &headers).await?;
    if prefix != expected {
        return Ok(wrong_warehouse());
    }
    let answer = call_catalog(
        &state,
        &method,
        &["v1", prefix.as_str(), "namespaces", namespace.as_str()],
        &query_for(&state, query.0.as_deref(), false),
        &setup,
        &headers,
    )
    .await?;
    if !answer.status.is_success() {
        return Ok(refused_by_catalog(answer.status));
    }
    Ok((answer.status, ApiJson(answer.body)))
}

/// `GET /api/iceberg/v1/{prefix}/namespaces/{namespace}/tables` — the
/// tables in `gold` (D2).
pub async fn tables(
    State(state): State<AppState>,
    AuthenticatedPrincipal(principal): AuthenticatedPrincipal,
    Path((prefix, namespace)): Path<(String, String)>,
    method: Method,
    headers: HeaderMap,
    query: RawQuery,
) -> ApiResult<DoorResponse> {
    let setup = setup(&state).await.map_err(not_set_up)?;
    if let Some(response) = refuse(&principal, Route::Tables, &method) {
        return Ok(response);
    }
    if !namespace_is_gold(&namespace) {
        return Ok(refused());
    }
    let expected = expected_prefix(&state, &setup, &headers).await?;
    if prefix != expected {
        return Ok(wrong_warehouse());
    }
    let answer = call_catalog(
        &state,
        &Method::GET,
        &[
            "v1",
            prefix.as_str(),
            "namespaces",
            namespace.as_str(),
            "tables",
        ],
        &query_for(&state, query.0.as_deref(), false),
        &setup,
        &headers,
    )
    .await?;
    if !answer.status.is_success() {
        return Ok(refused_by_catalog(answer.status));
    }
    Ok((answer.status, ApiJson(answer.body)))
}

/// `GET`/`HEAD /api/iceberg/v1/{prefix}/namespaces/{namespace}/tables/{table}`
/// — the table load (D2). The `GET` is COUNTED; the `HEAD` is the
/// "does it exist" probe and is not.
pub async fn table(
    State(state): State<AppState>,
    AuthenticatedPrincipal(principal): AuthenticatedPrincipal,
    Path((prefix, namespace, table)): Path<(String, String, String)>,
    method: Method,
    headers: HeaderMap,
    query: RawQuery,
) -> ApiResult<DoorResponse> {
    let setup = setup(&state).await.map_err(not_set_up)?;
    if let Some(response) = refuse(&principal, Route::Table, &method) {
        return Ok(response);
    }
    if !namespace_is_gold(&namespace) {
        return Ok(refused());
    }
    // The table name is checked before anything is forwarded
    // (DATA-5 S1 review BLOCKER 1): a name that carries a path
    // separator would walk out of the `gold` namespace.
    if !table_name_is_allowed(&table) {
        return Ok(refused());
    }
    let expected = expected_prefix(&state, &setup, &headers).await?;
    if prefix != expected {
        return Ok(wrong_warehouse());
    }
    let counted = method == Method::GET;
    let answer = call_catalog(
        &state,
        &method,
        &[
            "v1",
            prefix.as_str(),
            "namespaces",
            namespace.as_str(),
            "tables",
            table.as_str(),
        ],
        &query_for(&state, query.0.as_deref(), false),
        &setup,
        &headers,
    )
    .await?;
    if !answer.status.is_success() {
        return Ok(refused_by_catalog(answer.status));
    }
    let mut body = answer.body;
    rewrite_endpoint(&mut body, &setup.endpoint);
    if counted {
        // The count is written BEFORE the body is returned, and a count
        // that cannot be written refuses the load (ADR 0016 point 5).
        record_read(&state, &principal, &table).await?;
    }
    Ok((answer.status, ApiJson(body)))
}

/// `GET /api/iceberg/v1/{prefix}/namespaces/{namespace}/tables/{table}/credentials`
/// — the table's short-lived storage credentials (D2), with the published
/// storage address (D4).
pub async fn credentials(
    State(state): State<AppState>,
    AuthenticatedPrincipal(principal): AuthenticatedPrincipal,
    Path((prefix, namespace, table)): Path<(String, String, String)>,
    method: Method,
    headers: HeaderMap,
    query: RawQuery,
) -> ApiResult<DoorResponse> {
    let setup = setup(&state).await.map_err(not_set_up)?;
    if let Some(response) = refuse(&principal, Route::Credentials, &method) {
        return Ok(response);
    }
    if !namespace_is_gold(&namespace) {
        return Ok(refused());
    }
    // The table name is checked before anything is forwarded
    // (DATA-5 S1 review BLOCKER 1), for the same reason the table load
    // checks it: a name that carries a path separator would walk out of
    // the `gold` namespace.
    if !table_name_is_allowed(&table) {
        return Ok(refused());
    }
    let expected = expected_prefix(&state, &setup, &headers).await?;
    if prefix != expected {
        return Ok(wrong_warehouse());
    }
    let answer = call_catalog(
        &state,
        &Method::GET,
        &[
            "v1",
            prefix.as_str(),
            "namespaces",
            namespace.as_str(),
            "tables",
            table.as_str(),
            "credentials",
        ],
        &query_for(&state, query.0.as_deref(), false),
        &setup,
        &headers,
    )
    .await?;
    if !answer.status.is_success() {
        return Ok(refused_by_catalog(answer.status));
    }
    let mut body = answer.body;
    rewrite_endpoint(&mut body, &setup.endpoint);
    Ok((answer.status, ApiJson(body)))
}

/// The catch-all: any other method or path under `/api/iceberg` is
/// refused with the Iceberg error shape (D2), so a write never reaches
/// `Lakekeeper`.
pub async fn catch_all(
    State(state): State<AppState>,
    AuthenticatedPrincipal(principal): AuthenticatedPrincipal,
) -> ApiResult<DoorResponse> {
    // The deployment's own state is checked first, for the same reason
    // every other door route checks it: a deployment that has not
    // published its storage address has no door to answer with.
    let _ = setup(&state).await.map_err(not_set_up)?;
    // The caller is checked for the same reason, and then the answer is
    // the same one either way: the Iceberg refusal.
    if matches!(principal.id, PrincipalId::Service(_)) {
        return Ok(refused());
    }
    Ok(not_a_service())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// The plan's route table (D2): which method each route serves.
    #[test]
    fn the_served_methods_are_the_plan_s_table() {
        assert!(method_is_served(Route::Config, &Method::GET));
        assert!(!method_is_served(Route::Config, &Method::POST));

        assert!(method_is_served(Route::Namespaces, &Method::GET));
        assert!(!method_is_served(Route::Namespaces, &Method::POST));

        assert!(method_is_served(Route::Namespace, &Method::GET));
        assert!(method_is_served(Route::Namespace, &Method::HEAD));
        assert!(!method_is_served(Route::Namespace, &Method::POST));

        assert!(method_is_served(Route::Tables, &Method::GET));
        assert!(!method_is_served(Route::Tables, &Method::PUT));

        assert!(method_is_served(Route::Table, &Method::GET));
        assert!(method_is_served(Route::Table, &Method::HEAD));
        assert!(!method_is_served(Route::Table, &Method::DELETE));

        assert!(method_is_served(Route::Credentials, &Method::GET));
        assert!(!method_is_served(Route::Credentials, &Method::POST));
    }

    /// Only `gold` is served (ADR 0016 point 2).
    #[test]
    fn only_the_gold_namespace_is_served() {
        assert!(namespace_is_gold("gold"));
        assert!(!namespace_is_gold("bronze"));
        assert!(!namespace_is_gold("silver"));
        assert!(!namespace_is_gold("Gold"));
        assert!(!namespace_is_gold("gold.sales"));
    }

    /// The `s3.endpoint` rewrite replaces the vended address in the
    /// table's own `config` AND in each `storage-credentials[].config`,
    /// and leaves every other key alone (ADR 0016 point 4).
    #[test]
    fn the_vended_storage_address_is_replaced_in_both_places() {
        let mut body = json!({
            "metadata": {"format-version": 2},
            "config": {
                "s3.endpoint": "http://rustfs:9000/",
                "s3.path-style-access": "true",
                "s3.access-key-id": "AKIA",
            },
            "storage-credentials": [
                {
                    "prefix": "s3://lakehouse-warehouse/gold/orders/",
                    "config": {
                        "s3.endpoint": "http://rustfs:9000/",
                        "s3.access-key-id": "ASIA",
                        "s3.session-token": "token",
                    },
                },
            ],
        });
        rewrite_endpoint(&mut body, "https://storage.example.com");
        assert_eq!(
            body["config"]["s3.endpoint"],
            json!("https://storage.example.com")
        );
        assert_eq!(
            body["storage-credentials"][0]["config"]["s3.endpoint"],
            json!("https://storage.example.com")
        );
        // The keys are NOT replaced: they are still the catalog's
        // short-lived, table-scoped ones.
        assert_eq!(body["config"]["s3.access-key-id"], json!("AKIA"));
        assert_eq!(
            body["storage-credentials"][0]["config"]["s3.session-token"],
            json!("token")
        );
        assert_eq!(body["config"]["s3.path-style-access"], json!("true"));
        assert_eq!(body["metadata"]["format-version"], json!(2));
    }

    /// A profile that vends no `s3.endpoint` has no address to swap: the
    /// rewrite invents nothing.
    #[test]
    fn the_rewrite_invents_no_address_that_was_not_vended() {
        let mut body = json!({"config": {"s3.path-style-access": "true"}});
        rewrite_endpoint(&mut body, "https://storage.example.com");
        assert!(body["config"].get("s3.endpoint").is_none());
    }

    /// The catalog's own `uri` override is removed: a client that follows
    /// it would leave the door, and a read that leaves the door is a read
    /// nobody counts.
    #[test]
    fn the_catalogs_own_uri_override_is_removed() {
        let mut body = json!({
            "overrides": {
                "uri": "http://lakekeeper:8181/catalog",
                "idempotency-key-lifetime": "PT30M",
            },
            "defaults": {"prefix": "651eeefa-d506-11f1-b379-970f98828ea7"},
        });
        strip_uri_override(&mut body);
        assert!(body["overrides"].get("uri").is_none());
        assert_eq!(
            body["overrides"]["idempotency-key-lifetime"],
            json!("PT30M")
        );
        assert_eq!(
            body["defaults"]["prefix"],
            json!("651eeefa-d506-11f1-b379-970f98828ea7"),
            "the warehouse prefix the client must use is NOT removed"
        );
    }

    /// The namespace list is filtered to `gold` — and an empty list stays
    /// empty (the door never reports a namespace the catalog lacks).
    #[test]
    fn the_namespace_list_is_filtered_to_gold() {
        let mut body = json!({"namespaces": [["gold"], ["bronze"], ["silver", "sales"]]});
        filter_namespaces_to_gold(&mut body);
        assert_eq!(body["namespaces"], json!([["gold"]]));

        let mut empty = json!({"namespaces": []});
        filter_namespaces_to_gold(&mut empty);
        assert_eq!(empty["namespaces"], json!([]));
    }

    /// The "not set up" refusal names the setting that is missing, and
    /// says so in the fixed shape ADR 0016 point 4 asks for.
    #[test]
    fn the_not_set_up_refusal_names_the_missing_setting() {
        let err = not_set_up(NotSetUp::Endpoint);
        assert_eq!(err.status(), 503);
        assert!(err.to_string().contains("OUTSIDE_S3_ENDPOINT"));
        assert!(err.to_string().contains("not set up"));
    }

    /// Every reason the door can be "not set up" names what is missing —
    /// and the console's consumers route answers with the SAME sentence
    /// (plan D7), so the console and a tool are told the same thing.
    #[test]
    fn every_not_set_up_reason_names_what_is_missing() {
        assert!(NotSetUp::Endpoint.reason().contains("OUTSIDE_S3_ENDPOINT"));
        assert!(
            NotSetUp::TokenFile
                .reason()
                .contains("LAKEKEEPER_OUTSIDE_READ_TOKEN_FILE")
        );
        assert!(
            NotSetUp::TokenUnreadable
                .reason()
                .contains("LAKEKEEPER_OUTSIDE_READ_TOKEN_FILE")
        );
        for reason in [
            NotSetUp::Endpoint.reason(),
            NotSetUp::TokenFile.reason(),
            NotSetUp::TokenUnreadable.reason(),
        ] {
            assert!(
                reason.starts_with("Lakekeeper ") || reason.contains("not set up"),
                "the reason says what is wrong, in the fixed shape: {reason}"
            );
        }
    }

    /// The Iceberg error shape the door answers in: the spec's
    /// `{"error": {"message", "type", "code"}}`.
    #[test]
    fn the_refusal_carries_the_iceberg_error_shape() {
        let body = iceberg_error(REFUSED_MESSAGE, "ForbiddenException", 403);
        assert_eq!(body["error"]["message"], json!(REFUSED_MESSAGE));
        assert_eq!(body["error"]["type"], json!("ForbiddenException"));
        assert_eq!(body["error"]["code"], json!(403));
    }

    /// The table names the door forwards are the ones the Gold export
    /// writes; a name that could change the path's shape is refused
    /// (DATA-5 S1 review BLOCKER 1).
    #[test]
    fn only_a_plain_table_name_is_forwarded() {
        for allowed in ["orders", "gold_export_smoke", "orders-2026", "v2", "A1"] {
            assert!(table_name_is_allowed(allowed), "{allowed} is a real name");
        }
        for refused in [
            "",
            "..",
            ".",
            "../../silver/tables/x",
            "orders/../../silver",
            "orders%2F..%2F..",
            "orders%2F",
            "gold.orders",
            "gold orders",
            "orders;drop",
            "orders\n",
        ] {
            assert!(!table_name_is_allowed(refused), "{refused} must be refused");
        }
        // A name longer than the cap is not a name the export wrote.
        assert!(!table_name_is_allowed(&"a".repeat(MAX_TABLE_NAME_LEN + 1)));
        assert!(table_name_is_allowed(&"a".repeat(MAX_TABLE_NAME_LEN)));
    }

    /// The forwarded path is built segment by segment, so a segment can
    /// never become a separator or an escape (DATA-5 S1 review BLOCKER 1).
    #[test]
    fn the_forwarded_path_encodes_each_segment() {
        let url = forwarded_url(
            "http://lakekeeper:8181/catalog",
            &["v1", "gold", "tables", "../../silver/tables/x"],
        )
        .expect("the url is built");
        assert_eq!(
            url.path(),
            "/catalog/v1/gold/tables/..%2F..%2Fsilver%2Ftables%2Fx",
            "the segment is one segment: {url}"
        );
        assert!(
            !url.path().contains("/silver/"),
            "the path still holds no unencoded separator: {url}"
        );
    }

    /// The base address is the catalog's own, path and all: the door
    /// forwards under `/v1/...` after it.
    #[test]
    fn the_forwarded_path_keeps_the_catalogs_own_base_path() {
        let url = forwarded_url("http://lakekeeper:8181/catalog", &["v1", "config"])
            .expect("the url is built");
        assert_eq!(url.as_str(), "http://lakekeeper:8181/catalog/v1/config");

        // A base address with no path of its own.
        let url =
            forwarded_url("http://lakekeeper:8181", &["v1", "config"]).expect("the url is built");
        assert_eq!(url.as_str(), "http://lakekeeper:8181/v1/config");

        // A base address that ends in a slash does not get a doubled one.
        let url =
            forwarded_url("http://lakekeeper:8181/", &["v1", "config"]).expect("the url is built");
        assert_eq!(url.as_str(), "http://lakekeeper:8181/v1/config");
    }

    /// The `url` crate drops a segment that is exactly `.` or `..`, so
    /// the door refuses one rather than forward a path of another shape.
    #[test]
    fn a_dot_segment_is_never_forwarded() {
        for segment in [".", ".."] {
            let err = forwarded_url("http://lakekeeper:8181/catalog", &["v1", segment])
                .expect_err("a dot segment is refused");
            assert_eq!(err.status(), 503);
            assert!(
                !err.to_string().contains("lakekeeper"),
                "the answer never carries the catalog's own address: {err}"
            );
        }
    }

    /// A catalog address that is not a URL at all is the catalog's
    /// "unavailable" — not the parser's text.
    #[test]
    fn an_unparseable_catalog_address_is_the_catalogs_own_text() {
        let err = forwarded_url("not a url", &["v1", "config"]).expect_err("refused");
        assert_eq!(err.status(), 503);
        assert_eq!(err.to_string(), upstream_error::CATALOG.unavailable);
    }

    /// A session — even one holding everything — is refused: the count is
    /// per tool, and a session is not a tool (ADR 0016 point 1).
    #[test]
    fn a_session_principal_is_refused() {
        let mut principal = lakehouse_auth::Principal {
            id: lakehouse_auth::PrincipalId::User(uuid::Uuid::nil()),
            tenant_ids: Vec::new(),
            display_name: "A Session".to_owned(),
            permissions: lakehouse_auth::PermissionSet::parse("*:*"),
            provider: "local".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        };
        assert!(refuse(&principal, Route::Config, &Method::GET).is_some());

        principal.id = lakehouse_auth::PrincipalId::Service(uuid::Uuid::nil());
        assert!(refuse(&principal, Route::Config, &Method::GET).is_none());
        assert!(
            refuse(&principal, Route::Config, &Method::POST).is_some(),
            "a write is refused even for a service identity"
        );
    }
}
