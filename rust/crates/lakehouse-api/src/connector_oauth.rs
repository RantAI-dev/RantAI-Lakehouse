//! `OAuth2` for REST connectors, the API's half (`SRC-12` part C, decisions 6
//! to 8 and 12): the sign-in address with its PKCE challenge, the token
//! request (the code exchange, the refresh, and the connection test's), and
//! revocation.
//!
//! The routes that use it are `routes::connector_oauth`; the pending sign-in
//! rows are `lakehouse_store::connector_oauth`; the orchestrator's half is
//! `dagster/dispar_orchestrate/adapters/rest.py`.
//!
//! # What every request here guarantees
//!
//! - **The address is checked, then pinned.** A token or revocation address
//!   must be `https` (the operator's gate-only switch, `OAuthSettings`, may
//!   admit `http` for the acceptance gate's own provider), its host passes
//!   `connector_probe::resolve_checked` (the same private-address refusal as
//!   a connection test), and the client is pinned to the approved addresses
//!   with `connector_probe::pinned_http_client`. A connector's own address
//!   is therefore never dialled unchecked, and no part of the provider's
//!   answer is dialled at all.
//! - **No redirect is followed.** `pinned_http_client` builds the client with
//!   `redirect::Policy::none()`; a redirect is a failure here, because it
//!   would be a second target nobody checked.
//! - **The provider's words are never kept.** A failure is one of
//!   [`TokenFailure`]'s fixed texts. The answer's body is read only to find
//!   `access_token` and `refresh_token`; an `error` or `error_description` is
//!   not read, logged or stored (decision 12, principle 4).
//! - **Tokens live in [`SecretValue`]s** (no `Serialize`, no `Debug` leak) and
//!   in the request's memory only: an access token is never stored, and the
//!   refresh token is handed to the caller, who seals it
//!   (`connector_secret_store`).
//!
//! The client authenticates in the request body (`client_secret_post`, RFC
//! 6749 section 2.3.1) unless the caller asks for HTTP Basic
//! (`client_secret_basic`, section 2.3.1 too): `SRC-3` AD8 added it for the
//! business-app vendors that accept nothing else (`token_request_basic`,
//! [`revoke_request`]'s `client_auth`). The vendor table says which a vendor
//! needs (`saas_vendors::ClientAuth`); a `rest` connector always uses the body.

use std::time::Duration;

use base64::Engine as _;
use lakehouse_core::secret::SecretValue;
use lakehouse_store::saas_vendors::ClientAuth;
use serde::Deserialize;
use sha2::Digest as _;

use crate::connector_probe::{parse_endpoint_host_port, pinned_http_client, resolve_checked};
use crate::internal_hosts::InternalHosts;

/// Bound on one token or revocation request.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// The most bytes of a token answer read. A real answer is a few hundred
/// bytes; the cap keeps a hostile address from streaming without end.
const MAX_ANSWER_BYTES: usize = 64 * 1024;
/// The longest token accepted from a provider. The store accepts 16 KiB.
const MAX_TOKEN_BYTES: usize = 16 * 1024;

/// What an installation lets an `OAuth2` dial do. Built from the operator's
/// settings by `Config::oauth_settings`; the default is the safe one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OAuthSettings {
    /// Admit `http://` token and revocation addresses (`CONNECTOR_OAUTH_ALLOW_HTTP`,
    /// gate-only, default off).
    pub allow_http: bool,
}

/// Why a token or revocation request did not succeed. Every variant renders as
/// a fixed text ([`TokenFailure::message`]); none carries anything the
/// provider said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenFailure {
    /// The address was refused before any request: not `https`, not a plain
    /// address, or it resolves to a private/internal address. The text is the
    /// address check's own fixed sentence.
    Address(String),
    /// The provider answered 400, 401 or 403: the credential (the refresh
    /// token, the code, the client secret) is no longer accepted.
    Refused,
    /// The provider could not be reached, timed out, redirected, answered
    /// with another status, or did not answer with a token.
    Unavailable,
}

impl TokenFailure {
    /// The fixed text for this failure. Safe to show and to store.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Address(text) => text.clone(),
            Self::Refused => {
                "the provider refused the sign-in: the connector needs to sign in again".to_owned()
            }
            Self::Unavailable => {
                "the provider could not be reached, or its answer was not a token".to_owned()
            }
        }
    }
}

/// The tokens a successful token request returned.
#[derive(Debug)]
pub struct Tokens {
    /// Used for the request in hand and dropped. Never stored.
    pub access_token: SecretValue,
    /// Present when the provider issued one (the code exchange always should;
    /// a refresh only when it rotates).
    pub refresh_token: Option<SecretValue>,
}

/// The answer's two fields we read. No `deny_unknown_fields`: the rest of the
/// body is the provider's and is ignored, not kept.
#[derive(Deserialize)]
struct TokenBody {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
}

/// `BASE64URL(SHA256(verifier))`: the PKCE `S256` challenge (RFC 7636 section
/// 4.2).
#[must_use]
pub fn pkce_challenge(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier))
}

/// The lower-hex SHA-256 of a `state`: the only form in which it is stored
/// (`lakehouse_store::connector_oauth`). The `state` is 256 random bits, so a
/// plain digest is the right primitive (no password-hashing cost is needed).
#[must_use]
pub fn hash_state(state: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(64);
    for byte in sha2::Sha256::digest(state) {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// The provider's sign-in address with the query the flow needs.
///
/// An existing query on `authorize_url` (some providers need an `audience` or
/// `access_type`) is kept.
///
/// `None` when `authorize_url` does not parse, which the dial's validation
/// already refuses.
#[must_use]
pub fn authorize_url(
    authorize_url: &str,
    client_id: &str,
    redirect_uri: &str,
    scopes: &[String],
    state: &str,
    challenge: &str,
) -> Option<String> {
    let mut url = url::Url::parse(authorize_url).ok()?;
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("response_type", "code")
            .append_pair("client_id", client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("state", state)
            .append_pair("code_challenge", challenge)
            .append_pair("code_challenge_method", "S256");
        if !scopes.is_empty() {
            query.append_pair("scope", &scopes.join(" "));
        }
    }
    Some(url.into())
}

/// Whether `url` may be an `OAuth2` address on this installation: `https`, or
/// `http` when the operator's gate-only switch allows it.
#[must_use]
pub fn scheme_allowed(url: &str, settings: OAuthSettings) -> bool {
    url.starts_with("https://") || (settings.allow_http && url.starts_with("http://"))
}

const SCHEME_REFUSED: &str = "an OAuth address must start with https://";
const ADDRESS_REFUSED: &str = "an OAuth address must be a plain https:// address";

/// The save-time check of an address the product will dial: the scheme, and
/// the host through `resolve_checked`. Advisory, like `check_dial_ssrf`'s other
/// checks (the dial-time check in [`token_request`] is the authoritative one).
///
/// # Errors
///
/// A fixed sentence safe to show: the address check's own, or the two above.
pub async fn check_address(
    url: &str,
    internal_hosts: &InternalHosts,
    settings: OAuthSettings,
) -> Result<(), String> {
    if !scheme_allowed(url, settings) {
        return Err(SCHEME_REFUSED.to_owned());
    }
    let Some((host, port)) = parse_endpoint_host_port(url) else {
        return Err(ADDRESS_REFUSED.to_owned());
    };
    resolve_checked(host, port, internal_hosts)
        .await
        .map(|_| ())
}

/// Check `url` is an address the product may dial for `OAuth2` and return the
/// pinned client for it.
async fn client_for(
    url: &str,
    internal_hosts: &InternalHosts,
    settings: OAuthSettings,
) -> Result<reqwest::Client, TokenFailure> {
    if !scheme_allowed(url, settings) {
        return Err(TokenFailure::Address(SCHEME_REFUSED.to_owned()));
    }
    let Some((host, port)) = parse_endpoint_host_port(url) else {
        return Err(TokenFailure::Address(ADDRESS_REFUSED.to_owned()));
    };
    let approved = resolve_checked(host, port, internal_hosts)
        .await
        .map_err(TokenFailure::Address)?;
    pinned_http_client(url, host, port, &approved)
        .map_err(|_| TokenFailure::Address(ADDRESS_REFUSED.to_owned()))
}

/// Read at most [`MAX_ANSWER_BYTES`] of the answer; `None` when it is longer
/// or cannot be read.
async fn read_capped(mut response: reqwest::Response) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if body.len() + chunk.len() > MAX_ANSWER_BYTES {
                    return None;
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => return Some(body),
            Err(_) => return None,
        }
    }
}

/// `text` as RFC 6749 appendix B asks for a client id or secret that goes into
/// a Basic header: every byte outside `A-Za-z0-9-._~` is percent-encoded.
fn form_encode(text: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// The environment variable the acceptance gate sets to point a vendor at its mock
/// vendor server: a JSON object of vendor id to a bare `http(s)://host:port`.
pub const GATE_ORIGINS_ENV: &str = "SAAS_GATE_ORIGINS";

/// `url` with its scheme, host and port replaced by the gate's mock for `vendor_id`
/// (slice 2 review SHOULD-FIX 5; the Python twin is `saas.vendors.rebase_for_gate`).
///
/// This goes round "a vendor's hosts come only from the table" (`SRC-3` AD2), so it is
/// closed twice: nothing happens unless the gate-only switch
/// `CONNECTOR_OAUTH_ALLOW_HTTP=true` is on (`settings.allow_http`), and then only for a
/// vendor the variable names with a bare origin (no userinfo, path or query). Anything
/// else returns `url` unchanged, so a bad value fails closed to the real host.
#[must_use]
pub fn gate_rebase(settings: OAuthSettings, vendor_id: &str, url: &str) -> String {
    gate_rebase_with(
        settings,
        std::env::var(GATE_ORIGINS_ENV).ok().as_deref(),
        vendor_id,
        url,
    )
}

/// [`gate_rebase`] with the variable's value given, so a test needs no environment.
#[must_use]
pub fn gate_rebase_with(
    settings: OAuthSettings,
    origins: Option<&str>,
    vendor_id: &str,
    url: &str,
) -> String {
    let rebased = (|| {
        if !settings.allow_http {
            return None;
        }
        let table: serde_json::Value = serde_json::from_str(origins?).ok()?;
        let mock = reqwest::Url::parse(table.get(vendor_id)?.as_str()?).ok()?;
        let bare = matches!(mock.scheme(), "http" | "https")
            && mock.host_str().is_some()
            && mock.username().is_empty()
            && mock.password().is_none()
            && matches!(mock.path(), "" | "/")
            && mock.query().is_none();
        if !bare {
            return None;
        }
        let real = reqwest::Url::parse(url).ok()?;
        let mut out = mock;
        out.set_path(real.path());
        out.set_query(real.query());
        Some(out.to_string())
    })();
    rebased.unwrap_or_else(|| url.to_owned())
}

/// The `Authorization` header value of `client_secret_basic` (RFC 6749 2.3.1).
fn basic_header(client_id: &str, client_secret: &str) -> String {
    let pair = format!("{}:{}", form_encode(client_id), form_encode(client_secret));
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(pair)
    )
}

/// POST `form` to `url` and classify the status. `basic` is the client's id
/// and secret for `client_secret_basic`; with `None` the caller has put them
/// in `form` (or the request carries none).
async fn post_form(
    url: &str,
    form: &[(&str, &str)],
    basic: Option<(&str, &str)>,
    internal_hosts: &InternalHosts,
    settings: OAuthSettings,
) -> Result<reqwest::Response, TokenFailure> {
    post_fields(url, form, basic, false, internal_hosts, settings).await
}

/// [`post_form`], and with `json` the same fields as a JSON object body
/// (`client_secret_post_json`, slice 2 review BLOCKER 3: Zendesk's
/// `POST /oauth/tokens` "must be formatted as JSON",
/// <https://developer.zendesk.com/api-reference/ticketing/oauth/grant_type_tokens/>).
async fn post_fields(
    url: &str,
    form: &[(&str, &str)],
    basic: Option<(&str, &str)>,
    json: bool,
    internal_hosts: &InternalHosts,
    settings: OAuthSettings,
) -> Result<reqwest::Response, TokenFailure> {
    let client = client_for(url, internal_hosts, settings).await?;
    let request = client
        .post(url)
        .header(reqwest::header::ACCEPT, "application/json");
    let mut request = if json {
        let object: serde_json::Map<String, serde_json::Value> = form
            .iter()
            .map(|(k, v)| ((*k).to_owned(), serde_json::Value::String((*v).to_owned())))
            .collect();
        request.json(&object)
    } else {
        request.form(form)
    };
    if let Some((client_id, client_secret)) = basic {
        // The header carries the secret: it is built here, used once, and
        // dropped with the request, never logged.
        request = request.header(
            reqwest::header::AUTHORIZATION,
            basic_header(client_id, client_secret),
        );
    }
    let sent = tokio::time::timeout(REQUEST_TIMEOUT, request.send()).await;
    let response = match sent {
        Ok(Ok(response)) => response,
        Ok(Err(err)) => {
            // The class only (`is_connect`/`is_timeout`): a `reqwest` error's
            // text can carry the URL.
            tracing::warn!(
                connect = err.is_connect(),
                timeout = err.is_timeout(),
                "OAuth request failed"
            );
            return Err(TokenFailure::Unavailable);
        }
        Err(_) => {
            tracing::warn!("OAuth request timed out");
            return Err(TokenFailure::Unavailable);
        }
    };
    let status = response.status();
    if status.is_redirection() {
        tracing::warn!(
            status = status.as_u16(),
            "OAuth request was redirected; not followed"
        );
        return Err(TokenFailure::Unavailable);
    }
    if matches!(status.as_u16(), 400 | 401 | 403) {
        return Err(TokenFailure::Refused);
    }
    if !status.is_success() {
        tracing::warn!(
            status = status.as_u16(),
            "OAuth request answered with a failure status"
        );
        return Err(TokenFailure::Unavailable);
    }
    Ok(response)
}

/// A token as the store will accept it, or `None`.
fn token_value(raw: String) -> Option<SecretValue> {
    if raw.len() > MAX_TOKEN_BYTES
        || crate::connector_secret_store::validate_secret_value(&raw).is_err()
    {
        return None;
    }
    Some(SecretValue::new(raw))
}

/// Ask the token endpoint `url` for tokens with `form`: the code exchange
/// (`grant_type=authorization_code`) or the refresh (`grant_type=refresh_token`).
///
/// # Errors
///
/// A [`TokenFailure`]: the address was refused, the provider refused the
/// credential, or it could not be reached or did not answer with a token.
pub async fn token_request(
    url: &str,
    form: &[(&str, &str)],
    internal_hosts: &InternalHosts,
    settings: OAuthSettings,
) -> Result<Tokens, TokenFailure> {
    token_request_inner(url, form, None, false, internal_hosts, settings).await
}

/// [`token_request`] with the form sent as a JSON body (`client_secret_post_json`,
/// slice 2 review BLOCKER 3): `form` carries `client_id` and `client_secret` as for
/// `client_secret_post`; only the encoding differs.
///
/// # Errors
///
/// As [`token_request`].
pub async fn token_request_json(
    url: &str,
    form: &[(&str, &str)],
    internal_hosts: &InternalHosts,
    settings: OAuthSettings,
) -> Result<Tokens, TokenFailure> {
    token_request_inner(url, form, None, true, internal_hosts, settings).await
}

/// [`token_request`] with the client authenticating by HTTP Basic
/// (`client_secret_basic`, `SRC-3` AD8): `form` must not carry `client_id` or
/// `client_secret`; they go in the `Authorization` header.
///
/// # Errors
///
/// As [`token_request`].
pub async fn token_request_basic(
    url: &str,
    form: &[(&str, &str)],
    client_id: &str,
    client_secret: &str,
    internal_hosts: &InternalHosts,
    settings: OAuthSettings,
) -> Result<Tokens, TokenFailure> {
    token_request_inner(
        url,
        form,
        Some((client_id, client_secret)),
        false,
        internal_hosts,
        settings,
    )
    .await
}

async fn token_request_inner(
    url: &str,
    form: &[(&str, &str)],
    basic: Option<(&str, &str)>,
    json: bool,
    internal_hosts: &InternalHosts,
    settings: OAuthSettings,
) -> Result<Tokens, TokenFailure> {
    let response = post_fields(url, form, basic, json, internal_hosts, settings).await?;
    let Some(bytes) = read_capped(response).await else {
        tracing::warn!("OAuth token answer was too long or could not be read");
        return Err(TokenFailure::Unavailable);
    };
    let Ok(body) = serde_json::from_slice::<TokenBody>(&bytes) else {
        tracing::warn!("OAuth token answer was not the expected shape");
        return Err(TokenFailure::Unavailable);
    };
    let Some(access_token) = token_value(body.access_token) else {
        return Err(TokenFailure::Unavailable);
    };
    let refresh_token = match body.refresh_token {
        None => None,
        Some(raw) if raw.is_empty() => None,
        Some(raw) => match token_value(raw) {
            Some(value) => Some(value),
            None => return Err(TokenFailure::Unavailable),
        },
    };
    Ok(Tokens {
        access_token,
        refresh_token,
    })
}

/// Tell the provider to forget `token` (RFC 7009), a refresh token.
///
/// # Errors
///
/// As [`token_request`]. A provider answers 200 for a token it does not
/// know, so a `Refused` here means the client credentials were refused.
/// `client_auth` is how the client proves itself: in the body or by HTTP
/// Basic (`SRC-3` AD8).
pub async fn revoke_request(
    url: &str,
    token: &SecretValue,
    client_id: &str,
    client_secret: &SecretValue,
    client_auth: ClientAuth,
    internal_hosts: &InternalHosts,
    settings: OAuthSettings,
) -> Result<(), TokenFailure> {
    let token_fields = [
        ("token", token.expose_secret()),
        ("token_type_hint", "refresh_token"),
    ];
    match client_auth {
        // `PostJson` has no revocation address in the table (Zendesk revokes with a
        // DELETE, not RFC 7009); if one is ever added it is sent as JSON like the
        // token request.
        ClientAuth::Post | ClientAuth::PostJson => {
            post_fields(
                url,
                &[
                    token_fields[0],
                    token_fields[1],
                    ("client_id", client_id),
                    ("client_secret", client_secret.expose_secret()),
                ],
                None,
                client_auth == ClientAuth::PostJson,
                internal_hosts,
                settings,
            )
            .await
        }
        ClientAuth::Basic => {
            post_form(
                url,
                &token_fields,
                Some((client_id, client_secret.expose_secret())),
                internal_hosts,
                settings,
            )
            .await
        }
    }
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use wiremock::matchers::{body_json, body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const HTTP: OAuthSettings = OAuthSettings { allow_http: true };

    #[test]
    fn the_challenge_is_the_s256_of_the_verifier_rfc_7636_appendix_b() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn the_stored_form_of_a_state_is_its_sha256_in_lower_hex() {
        assert_eq!(
            hash_state("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn the_sign_in_address_carries_the_flow_and_keeps_an_existing_query() {
        let url = authorize_url(
            "https://login.example.com/authorize?audience=api",
            "app 1",
            "https://console.example.com/api/connectors/oauth/callback",
            &["read".to_owned(), "offline_access".to_owned()],
            "STATE",
            "CHALLENGE",
        )
        .unwrap();
        let parsed = url::Url::parse(&url).unwrap();
        let pairs: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(pairs["audience"], "api");
        assert_eq!(pairs["response_type"], "code");
        assert_eq!(pairs["client_id"], "app 1");
        assert_eq!(pairs["state"], "STATE");
        assert_eq!(pairs["code_challenge"], "CHALLENGE");
        assert_eq!(pairs["code_challenge_method"], "S256");
        assert_eq!(pairs["scope"], "read offline_access");
        assert_eq!(
            pairs["redirect_uri"],
            "https://console.example.com/api/connectors/oauth/callback"
        );
        assert!(!pairs.contains_key("client_secret"));
        assert!(!pairs.contains_key("code_verifier"));
    }

    #[test]
    fn no_scope_parameter_is_sent_when_none_is_asked() {
        let url =
            authorize_url("https://l.example.com/a", "c", "https://r", &[], "s", "c").unwrap();
        assert!(!url.contains("scope="));
    }

    #[tokio::test]
    async fn an_http_address_is_refused_unless_the_gate_switch_is_on() {
        let server = MockServer::start().await;
        let url = format!("{}/token", server.uri());
        let refused = token_request(
            &url,
            &[("a", "b")],
            &InternalHosts::ALL,
            OAuthSettings::default(),
        )
        .await;
        assert!(
            matches!(refused, Err(TokenFailure::Address(_))),
            "{refused:?}"
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_private_address_is_refused_before_any_request() {
        let server = MockServer::start().await;
        let url = format!("{}/token", server.uri());
        let refused = token_request(&url, &[("a", "b")], &InternalHosts::NONE, HTTP).await;
        assert!(
            matches!(refused, Err(TokenFailure::Address(_))),
            "{refused:?}"
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[test]
    fn the_gate_rebase_is_ignored_unless_the_gate_only_switch_is_on_and_the_vendor_named() {
        let origins = Some(r#"{"zendesk": "http://saas-stub-g6:8102"}"#);
        let real = "https://acme.zendesk.com/oauth/tokens?x=1";
        let off = OAuthSettings { allow_http: false };
        assert_eq!(gate_rebase_with(off, origins, "zendesk", real), real);
        assert_eq!(
            gate_rebase_with(HTTP, origins, "zendesk", real),
            "http://saas-stub-g6:8102/oauth/tokens?x=1"
        );
        assert_eq!(gate_rebase_with(HTTP, origins, "hubspot", real), real);
        for odd in [
            "not json",
            "[]",
            r#"{"zendesk": 5}"#,
            r#"{"zendesk": "ftp://x:1"}"#,
            r#"{"zendesk": "http://u:p@x:1"}"#,
            r#"{"zendesk": "http://x:1/prefix"}"#,
            r#"{"zendesk": "http://x:1/?q=1"}"#,
        ] {
            assert_eq!(
                gate_rebase_with(HTTP, Some(odd), "zendesk", real),
                real,
                "{odd}"
            );
        }
        assert_eq!(gate_rebase_with(HTTP, None, "zendesk", real), real);
    }

    #[tokio::test]
    async fn post_json_client_authentication_sends_the_same_fields_as_a_json_body() {
        // Slice 2 review BLOCKER 3: Zendesk's token endpoint "must be formatted as JSON".
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(header("content-type", "application/json"))
            .and(body_json(serde_json::json!({
                "grant_type": "refresh_token", "refresh_token": "refresh-1",
                "client_id": "app", "client_secret": "s3cret"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "access-1", "refresh_token": "refresh-2"
            })))
            .expect(1)
            .mount(&server)
            .await;
        let tokens = token_request_json(
            &format!("{}/token", server.uri()),
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", "refresh-1"),
                ("client_id", "app"),
                ("client_secret", "s3cret"),
            ],
            &InternalHosts::ALL,
            HTTP,
        )
        .await
        .unwrap();
        assert_eq!(tokens.refresh_token.unwrap().expose_secret(), "refresh-2");
    }

    #[tokio::test]
    async fn a_token_answer_gives_both_tokens_and_sends_the_form_in_the_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .and(body_string_contains("client_secret=s3cret"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "access-1", "refresh_token": "refresh-2",
                "token_type": "bearer", "expires_in": 5
            })))
            .expect(1)
            .mount(&server)
            .await;
        let tokens = token_request(
            &format!("{}/token", server.uri()),
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", "refresh-1"),
                ("client_id", "app"),
                ("client_secret", "s3cret"),
            ],
            &InternalHosts::ALL,
            HTTP,
        )
        .await
        .unwrap();
        assert_eq!(tokens.access_token.expose_secret(), "access-1");
        assert_eq!(tokens.refresh_token.unwrap().expose_secret(), "refresh-2");
        let seen = server.received_requests().await.unwrap();
        assert_eq!(
            seen[0].headers.get("accept").unwrap().to_str().unwrap(),
            "application/json"
        );
    }

    #[tokio::test]
    async fn an_answer_without_a_refresh_token_has_none() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "access-1"
            })))
            .mount(&server)
            .await;
        let tokens = token_request(
            &format!("{}/token", server.uri()),
            &[],
            &InternalHosts::ALL,
            HTTP,
        )
        .await
        .unwrap();
        assert!(tokens.refresh_token.is_none());
    }

    #[tokio::test]
    async fn a_refusal_is_a_fixed_text_and_the_providers_words_never_appear() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "error": "invalid_grant",
                "error_description": "PROVIDER-WORDS refresh token expired for user@example.com"
            })))
            .mount(&server)
            .await;
        let err = token_request(
            &format!("{}/token", server.uri()),
            &[],
            &InternalHosts::ALL,
            HTTP,
        )
        .await
        .unwrap_err();
        assert_eq!(err, TokenFailure::Refused);
        let text = format!("{err:?} {}", err.message());
        assert!(!text.contains("PROVIDER-WORDS") && !text.contains("user@example.com"));
    }

    #[tokio::test]
    async fn other_failures_a_redirect_and_a_wrong_shape_are_unavailable() {
        for response in [
            ResponseTemplate::new(500).set_body_string("PROVIDER-WORDS"),
            ResponseTemplate::new(302).insert_header("location", "http://169.254.169.254/"),
            ResponseTemplate::new(200).set_body_string("not json PROVIDER-WORDS"),
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "access_token": "" })),
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "refresh_token": "x" })),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(response)
                .mount(&server)
                .await;
            let err = token_request(
                &format!("{}/token", server.uri()),
                &[],
                &InternalHosts::ALL,
                HTTP,
            )
            .await
            .unwrap_err();
            assert_eq!(err, TokenFailure::Unavailable);
            assert!(!err.message().contains("PROVIDER-WORDS"));
            // A redirect is never followed: the one request is all there is.
            assert_eq!(server.received_requests().await.unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn revocation_sends_the_token_and_the_client_and_succeeds_on_200() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/revoke"))
            .and(body_string_contains("token=refresh-1"))
            .and(body_string_contains("token_type_hint=refresh_token"))
            .and(body_string_contains("client_id=app"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        revoke_request(
            &format!("{}/revoke", server.uri()),
            &SecretValue::new("refresh-1"),
            "app",
            &SecretValue::new("s3cret"),
            ClientAuth::Post,
            &InternalHosts::ALL,
            HTTP,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_failed_revocation_is_a_fixed_text() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503).set_body_string("PROVIDER-WORDS"))
            .mount(&server)
            .await;
        let err = revoke_request(
            &format!("{}/revoke", server.uri()),
            &SecretValue::new("refresh-1"),
            "app",
            &SecretValue::new("s3cret"),
            ClientAuth::Post,
            &InternalHosts::ALL,
            HTTP,
        )
        .await
        .unwrap_err();
        assert_eq!(err, TokenFailure::Unavailable);
    }

    /// `SRC-3` AD8: `client_secret_basic` puts the client in the header, never
    /// in the body, and encodes it as RFC 6749 appendix B asks.
    #[tokio::test]
    async fn basic_client_authentication_sends_the_client_in_the_header_only() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "access-1"
            })))
            .expect(1)
            .mount(&server)
            .await;
        token_request_basic(
            &format!("{}/token", server.uri()),
            &[("grant_type", "refresh_token"), ("refresh_token", "r")],
            "app id",
            "s3cret:+",
            &InternalHosts::ALL,
            HTTP,
        )
        .await
        .unwrap();
        let seen = server.received_requests().await.unwrap();
        let body = String::from_utf8_lossy(&seen[0].body).into_owned();
        assert!(
            !body.contains("client_id") && !body.contains("client_secret"),
            "{body}"
        );
        let header = seen[0]
            .headers
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(header.strip_prefix("Basic ").unwrap())
            .unwrap();
        assert_eq!(String::from_utf8(decoded).unwrap(), "app%20id:s3cret%3A%2B");
    }

    #[tokio::test]
    async fn basic_revocation_sends_the_client_in_the_header_only() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/revoke"))
            .and(body_string_contains("token=refresh-1"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        revoke_request(
            &format!("{}/revoke", server.uri()),
            &SecretValue::new("refresh-1"),
            "app",
            &SecretValue::new("s3cret"),
            ClientAuth::Basic,
            &InternalHosts::ALL,
            HTTP,
        )
        .await
        .unwrap();
        let seen = server.received_requests().await.unwrap();
        let body = String::from_utf8_lossy(&seen[0].body).into_owned();
        assert!(!body.contains("client_secret"), "{body}");
        assert!(seen[0].headers.get("authorization").is_some());
    }
}
