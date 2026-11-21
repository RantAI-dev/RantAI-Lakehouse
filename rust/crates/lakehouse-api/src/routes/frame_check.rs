//! `GET /api/dashboard/frame-check?url=` — whether a web-page card's site
//! allows being shown inside a dashboard (`BI-72`, decision 3).
//!
//! A browser does not tell a page that a cross-origin frame was refused, so
//! the console cannot see it and a refused card was an empty white box. The
//! server can: it reads the site's own `X-Frame-Options` and `Content-Security-Policy`
//! `frame-ancestors` and answers `{ "framable": true | false | null }`.
//!
//! - `false`: a header says the console may not frame the site
//!   (`X-Frame-Options: DENY` or `SAMEORIGIN`, or a `frame-ancestors` list
//!   that is `'none'` or does not include the console's own origin).
//! - `true`: no header refuses, or a list names the console.
//! - `null`: no claim (principle 2): the site could not be reached, answered
//!   with a redirect or an error, or a `frame-ancestors` list needs the
//!   console's origin and `CONSOLE_PUBLIC_URL` is not set.
//!
//! The route answers only for an address on the deployment's content sites,
//! so it is no way to probe an arbitrary host. It dials through
//! [`crate::connector_probe::pinned_https_client`] (the same guarded path as
//! the print view's image fetch: `resolve_checked`, no redirect, a short
//! timeout), sends a `HEAD` (a `GET` when `HEAD` is not answered with a
//! success) and never reads a body. The upstream's own text never reaches the
//! answer (SEC-11): the verdict is a boolean. Answers are kept per address for
//! ten minutes in memory.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Query, State};
use lakehouse_core::ApiError;
use reqwest::header::{CONTENT_SECURITY_POLICY, HeaderMap};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::state::AppState;

/// How long one request to the site may take.
const CHECK_TIMEOUT: Duration = Duration::from_secs(5);

/// How long an answer is kept for an address.
const CACHE_TTL: Duration = Duration::from_secs(10 * 60);

/// The most addresses kept; the cache is emptied when it is full, so a caller
/// cannot grow it without bound.
const CACHE_MAX: usize = 512;

/// What an address off the content sites, or no address, gets.
const NOT_ALLOWED: &str = "url must be an address on an allowed content site.";

static CACHE: LazyLock<Mutex<HashMap<String, (Instant, Option<bool>)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The query of `GET /api/dashboard/frame-check`. Optional so a missing
/// address is the route's own 400.
#[derive(Debug, Deserialize)]
pub struct FrameCheckQuery {
    /// The address saved on the web-page card.
    pub url: Option<String>,
}

/// An origin as the comparison needs it: scheme, host and effective port.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Origin {
    scheme: String,
    host: String,
    port: u16,
}

impl Origin {
    /// The origin of an `http(s)` address; `None` for anything else.
    fn parse(url: &str) -> Option<Self> {
        let (scheme, rest) = url.trim().split_once("://")?;
        let scheme = scheme.to_ascii_lowercase();
        let default_port = match scheme.as_str() {
            "https" => 443,
            "http" => 80,
            _ => return None,
        };
        let authority = rest.split(['/', '?', '#']).next()?;
        if authority.is_empty() || authority.contains('@') {
            return None;
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => (host, port.parse().ok()?),
            None => (authority, default_port),
        };
        Some(Self {
            scheme,
            host: host.to_ascii_lowercase(),
            port,
        })
    }
}

/// Whether one `frame-ancestors` source expression names `console`.
fn source_names(source: &str, console: &Origin) -> bool {
    let source = source.to_ascii_lowercase();
    if source == "*" {
        return true;
    }
    if let Some(scheme) = source.strip_suffix(':') {
        return scheme == console.scheme;
    }
    let (scheme, rest) = match source.split_once("://") {
        Some((scheme, rest)) => (Some(scheme), rest),
        None => (None, source.as_str()),
    };
    if scheme.is_some_and(|s| s != console.scheme) {
        return false;
    }
    let authority = rest.split('/').next().unwrap_or_default();
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    };
    let host_matches = match host.strip_prefix("*.") {
        Some(suffix) => console.host.ends_with(&format!(".{suffix}")),
        None => host == console.host,
    };
    let default_port = if console.scheme == "https" { 443 } else { 80 };
    let port_matches = match port {
        Some("*") => true,
        Some(port) => port.parse::<u16>().ok() == Some(console.port),
        None => console.port == default_port,
    };
    host_matches && port_matches
}

/// The answer of one `frame-ancestors` directive's sources.
fn ancestors_allow(sources: &[&str], site: &Origin, console: Option<&Origin>) -> Option<bool> {
    let lowered: Vec<String> = sources.iter().map(|s| s.to_ascii_lowercase()).collect();
    if lowered.iter().any(|s| s == "'none'") {
        return Some(false);
    }
    // `'self'` is the site itself: it names the console only when the console
    // is the site.
    if lowered.iter().any(|s| s == "'self'") && console == Some(site) {
        return Some(true);
    }
    let Some(console) = console else {
        // Only `*` can be judged without the console's origin.
        return lowered.iter().any(|s| s == "*").then_some(true);
    };
    Some(lowered.iter().any(|s| source_names(s, console)))
}

/// Whether the console may frame a site that answered with `headers`:
/// `Some(false)` when a header refuses it, `Some(true)` when none does and the
/// ones that list ancestors name the console, `None` when that cannot be told.
fn verdict(headers: &HeaderMap, site: &Origin, console: Option<&Origin>) -> Option<bool> {
    for value in headers.get_all("x-frame-options") {
        let Ok(text) = value.to_str() else { continue };
        for part in text.split(',') {
            match part.trim().to_ascii_uppercase().as_str() {
                "DENY" => return Some(false),
                "SAMEORIGIN" if console != Some(site) => return Some(false),
                // `ALLOW-FROM` is not read by current browsers.
                _ => {}
            }
        }
    }
    let mut unknown = false;
    for value in headers.get_all(CONTENT_SECURITY_POLICY) {
        let Ok(text) = value.to_str() else { continue };
        // One header can hold several comma-separated policies; each applies.
        for policy in text.split(',') {
            for directive in policy.split(';') {
                let mut words = directive.split_whitespace();
                if !words
                    .next()
                    .is_some_and(|name| name.eq_ignore_ascii_case("frame-ancestors"))
                {
                    continue;
                }
                let sources: Vec<&str> = words.collect();
                match ancestors_allow(&sources, site, console) {
                    Some(false) => return Some(false),
                    Some(true) => {}
                    None => unknown = true,
                }
            }
        }
    }
    if unknown { None } else { Some(true) }
}

/// Ask `url` for its headers and judge them: a `HEAD`, then a `GET` (its body
/// never read) when `HEAD` is not answered with a success. A redirect, an
/// error status or no answer is no claim.
async fn probe(
    client: &reqwest::Client,
    url: &str,
    site: &Origin,
    console: Option<&Origin>,
) -> Option<bool> {
    let mut response = client.head(url).send().await.ok();
    if !response.as_ref().is_some_and(|r| r.status().is_success()) {
        response = client.get(url).send().await.ok();
    }
    let response = response.filter(|r| r.status().is_success())?;
    verdict(response.headers(), site, console)
}

fn cached(url: &str) -> Option<Option<bool>> {
    let cache = CACHE.lock().ok()?;
    let (at, answer) = cache.get(url)?;
    (at.elapsed() < CACHE_TTL).then_some(*answer)
}

fn remember(url: &str, answer: Option<bool>) {
    let Ok(mut cache) = CACHE.lock() else { return };
    if cache.len() >= CACHE_MAX {
        cache.retain(|_, (at, _)| at.elapsed() < CACHE_TTL);
        if cache.len() >= CACHE_MAX {
            cache.clear();
        }
    }
    cache.insert(url.to_owned(), (Instant::now(), answer));
}

/// `GET /api/dashboard/frame-check?url=` — see the module documentation.
///
/// # Errors
///
/// 400 for an address that is missing or not on the content sites; 403 for a
/// caller without `dashboard:read`; the classified store error when the
/// content sites cannot be read. A site that cannot be reached is not an
/// error: the answer is `{ "framable": null }`.
pub async fn frame_check(
    State(state): State<AppState>,
    Query(query): Query<FrameCheckQuery>,
) -> ApiResult<ApiJson<Value>> {
    let url = query.url.unwrap_or_default();
    let not_allowed = || ApiError::BadRequest(NOT_ALLOWED.to_owned());
    let site = lakehouse_bi::embed_access::https_origin_of(&url).ok_or_else(not_allowed)?;
    let format = crate::routes::settings::format_settings(&state).await?;
    if !format.content_sites.contains(&site) {
        return Err(not_allowed().into());
    }
    if let Some(answer) = cached(&url) {
        return Ok(ApiJson(json!({ "framable": answer })));
    }
    let answer = match (
        Origin::parse(&site),
        crate::connector_probe::pinned_https_client(
            &url,
            &state.config.connector_internal_hosts(),
            CHECK_TIMEOUT,
        )
        .await,
    ) {
        (Some(site), Ok(client)) => {
            let console = state
                .config
                .console_public_url
                .as_deref()
                .and_then(Origin::parse);
            probe(&client, &url, &site, console.as_ref()).await
        }
        // A host the guard refuses or that did not resolve: no claim.
        _ => None,
    };
    remember(&url, answer);
    Ok(ApiJson(json!({ "framable": answer })))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use reqwest::header::HeaderValue;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn console() -> Origin {
        Origin::parse("https://lake.example.com").unwrap()
    }

    fn site() -> Origin {
        Origin::parse("https://status.example.org").unwrap()
    }

    fn with(name: &'static str, value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(name, HeaderValue::from_str(value).unwrap());
        headers
    }

    #[test]
    fn deny_and_sameorigin_refuse_the_console() {
        let c = console();
        assert_eq!(
            verdict(&with("x-frame-options", "DENY"), &site(), Some(&c)),
            Some(false)
        );
        assert_eq!(
            verdict(&with("x-frame-options", "sameorigin"), &site(), Some(&c)),
            Some(false)
        );
        assert_eq!(
            verdict(&with("x-frame-options", "SAMEORIGIN"), &site(), None),
            Some(false)
        );
    }

    #[test]
    fn frame_ancestors_none_refuses_and_a_list_needs_the_console() {
        let c = console();
        let csp = |v: &str| verdict(&with("content-security-policy", v), &site(), Some(&c));
        assert_eq!(
            csp("default-src 'self'; frame-ancestors 'none'"),
            Some(false)
        );
        assert_eq!(csp("frame-ancestors 'self'"), Some(false));
        assert_eq!(csp("frame-ancestors https://other.example"), Some(false));
        assert_eq!(
            csp("frame-ancestors https://other.example https://lake.example.com"),
            Some(true)
        );
        assert_eq!(csp("frame-ancestors *.example.com"), Some(true));
        assert_eq!(
            csp("frame-ancestors https://lake.example.com:8443"),
            Some(false)
        );
        assert_eq!(csp("frame-ancestors *"), Some(true));
        assert_eq!(csp("frame-ancestors https:"), Some(true));
        assert_eq!(
            csp("default-src 'self'"),
            Some(true),
            "no frame-ancestors: no refusal"
        );
    }

    #[test]
    fn no_header_means_framable_and_an_unknown_console_means_no_claim_for_a_list() {
        assert_eq!(verdict(&HeaderMap::new(), &site(), None), Some(true));
        let list = with(
            "content-security-policy",
            "frame-ancestors https://lake.example.com",
        );
        assert_eq!(verdict(&list, &site(), None), None);
        let none = with("content-security-policy", "frame-ancestors 'none'");
        assert_eq!(verdict(&none, &site(), None), Some(false));
        let any = with("content-security-policy", "frame-ancestors *");
        assert_eq!(verdict(&any, &site(), None), Some(true));
    }

    async fn probe_with(headers: &[(&str, &str)], head_status: u16) -> Option<bool> {
        let server = MockServer::start().await;
        let mut head = ResponseTemplate::new(head_status);
        let mut get = ResponseTemplate::new(200);
        for (k, v) in headers {
            head = head.insert_header(*k, *v);
            get = get.insert_header(*k, *v);
        }
        Mock::given(method("HEAD"))
            .respond_with(head)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .respond_with(get)
            .mount(&server)
            .await;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        probe(&client, &server.uri(), &site(), Some(&console())).await
    }

    #[tokio::test]
    async fn a_site_that_sends_deny_is_not_framable() {
        assert_eq!(
            probe_with(&[("X-Frame-Options", "DENY")], 200).await,
            Some(false)
        );
    }

    #[tokio::test]
    async fn a_site_that_sends_sameorigin_is_not_framable() {
        assert_eq!(
            probe_with(&[("X-Frame-Options", "SAMEORIGIN")], 200).await,
            Some(false)
        );
    }

    #[tokio::test]
    async fn a_site_with_frame_ancestors_none_is_not_framable() {
        let h = [("Content-Security-Policy", "frame-ancestors 'none'")];
        assert_eq!(probe_with(&h, 200).await, Some(false));
    }

    #[tokio::test]
    async fn a_list_with_the_console_is_framable_and_without_it_is_not() {
        let with_console = [(
            "Content-Security-Policy",
            "frame-ancestors https://lake.example.com",
        )];
        assert_eq!(probe_with(&with_console, 200).await, Some(true));
        let without = [(
            "Content-Security-Policy",
            "frame-ancestors https://other.example",
        )];
        assert_eq!(probe_with(&without, 200).await, Some(false));
    }

    #[tokio::test]
    async fn a_site_with_no_header_is_framable() {
        assert_eq!(probe_with(&[], 200).await, Some(true));
    }

    #[tokio::test]
    async fn a_head_that_is_refused_falls_back_to_get() {
        let h = [("X-Frame-Options", "DENY")];
        assert_eq!(probe_with(&h, 405).await, Some(false));
    }

    #[tokio::test]
    async fn an_unreachable_site_is_no_claim() {
        let server = MockServer::start().await;
        let url = server.uri();
        drop(server);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        assert_eq!(probe(&client, &url, &site(), Some(&console())).await, None);
    }

    #[tokio::test]
    async fn a_redirect_is_no_claim() {
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::any())
            .respond_with(
                ResponseTemplate::new(302).insert_header("Location", "https://elsewhere.example/"),
            )
            .mount(&server)
            .await;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        assert_eq!(
            probe(&client, &server.uri(), &site(), Some(&console())).await,
            None
        );
    }
}
