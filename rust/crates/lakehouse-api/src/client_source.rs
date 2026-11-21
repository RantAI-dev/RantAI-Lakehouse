//! The source of a request: the address it came from (`SEC-75`, plan
//! `adm-130`, decision 1).
//!
//! # Why
//!
//! The API is served behind the console's own `/api` proxy, so the peer
//! address of a request is the proxy's, not the browser's. Without a
//! trusted source the API cannot tell one caller from another, and every
//! limit it applies is one limit for everybody.
//!
//! # The rule
//!
//! One function owns the answer ([`resolve`]):
//!
//! * A peer the operator did not list in `TRUSTED_PROXIES` IS the
//!   source. Its `X-Forwarded-For` header is ignored, whatever it says:
//!   only a trusted peer may name a source.
//! * A peer in the trusted list may name the source in
//!   `X-Forwarded-For`. The source is then the RIGHT-MOST entry of that
//!   header that is not itself trusted. Proxies append the address they
//!   saw, so the right-most entry is the address the caller had, and
//!   anything to its left was written by the caller.
//! * No header, no usable header, or a header whose entries are all
//!   trusted: the source is the peer.
//! * An empty trusted list trusts no header at all (fail closed).
//!
//! # What the header is NOT
//!
//! The header decides the source and nothing else. It is not an
//! authorization input, and it never reaches a decision the peer's own
//! address would not reach.

use std::net::{IpAddr, SocketAddr};

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::request::Parts;
use ipnet::IpNet;

use crate::state::AppState;

/// The header a proxy uses to name the address it saw.
///
/// `http` 1.x has no constant for it (`FORWARDED` is the RFC 7239
/// header, a different one), so the name is spelled out.
const FORWARDED_FOR: &str = "x-forwarded-for";

/// The source of a request whose peer address the API cannot see.
///
/// One fixed bucket for every such caller, never a random or an empty
/// string: a bucket per caller would be a bucket per request, and the
/// per-source limit would count nothing.
pub const UNKNOWN_SOURCE: &str = "unknown";

/// The proxies whose `X-Forwarded-For` the API trusts.
///
/// Two kinds of entry, because a compose network's addresses are not
/// fixed: an address or a network stays an address or a network, while a
/// compose service NAME is resolved again every few minutes (see
/// [`TrustedProxies::refresh`]) — a container that is recreated comes
/// back with another address.
#[derive(Debug, Clone, Default)]
pub struct TrustedProxies {
    /// The networks the operator listed (`TRUSTED_PROXIES`).
    networks: Vec<IpNet>,
    /// The service names the operator listed (`TRUSTED_PROXIES`).
    service_names: Vec<String>,
    /// The addresses the service names resolved to, last time.
    resolved: Vec<IpAddr>,
}

impl TrustedProxies {
    /// The proxies of a deployment: the listed networks and the listed
    /// service names.
    #[must_use]
    pub fn new(networks: Vec<IpNet>, service_names: Vec<String>) -> Self {
        Self {
            networks,
            service_names,
            resolved: Vec::new(),
        }
    }

    /// Whether `ip` is one of the trusted proxies.
    ///
    /// The address is canonicalised first (an IPv4-mapped IPv6 address
    /// is the IPv4 address it wraps), so `::ffff:192.168.18.5` matches
    /// the network `192.168.18.0/24` — the same rule the connector
    /// allowlist uses (`internal_hosts::canonical`).
    #[must_use]
    pub fn is_trusted(&self, ip: &IpAddr) -> bool {
        let ip = crate::internal_hosts::canonical(ip);
        self.networks.iter().any(|net| net.contains(&ip)) || self.resolved.contains(&ip)
    }

    /// Resolve the service names again, now.
    ///
    /// A name that does not resolve is logged and skipped: the proxy is
    /// then not trusted until the next refresh, which is the fail-closed
    /// direction (the header of an unresolvable proxy is not read).
    pub async fn refresh(&mut self) {
        let mut resolved = Vec::new();
        for name in &self.service_names {
            match tokio::net::lookup_host((name.as_str(), 80_u16)).await {
                Ok(addresses) => resolved.extend(
                    addresses.map(|address| crate::internal_hosts::canonical(&address.ip())),
                ),
                Err(err) => {
                    tracing::warn!(
                        %name,
                        %err,
                        "a trusted proxy's service name did not resolve; its \
                         X-Forwarded-For is not read until the next refresh"
                    );
                }
            }
        }
        self.resolved = resolved;
    }
}

/// Split `TRUSTED_PROXIES` into the networks it lists and the service
/// names it lists.
///
/// An entry that parses as a network or an address is one (parsed with
/// the existing `internal_hosts::parse_cidrs`, so a network written with
/// host bits set is refused the same way the connector allowlist refuses
/// it). An entry that is not one is a service name when it looks like
/// one (`console`, `lakehouse-frontend`).
///
/// # Errors
///
/// Returns a message naming the first entry that is neither a network, an
/// address, nor a service name. A dropped entry would quietly stop
/// trusting a proxy the operator meant to trust, so the list is refused
/// instead.
pub fn parse_trusted_proxies(raw: &str) -> Result<(Vec<IpNet>, Vec<String>), String> {
    let mut networks = Vec::new();
    let mut service_names = Vec::new();
    for entry in raw
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|entry| !entry.is_empty())
    {
        match crate::internal_hosts::parse_cidrs(entry) {
            Ok(parsed) => networks.extend(parsed),
            Err(_) if looks_like_service_name(entry) => service_names.push(entry.to_owned()),
            Err(err) => {
                return Err(format!(
                    "{entry:?} is neither a network, an address, nor a service name: {err}"
                ));
            }
        }
    }
    Ok((networks, service_names))
}

/// Whether `entry` has the shape of a container or host name: letters,
/// digits, `.`, `-` and `_`, and at least one letter (so a name is never
/// a number that was meant to be an address).
fn looks_like_service_name(entry: &str) -> bool {
    entry
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        && entry.chars().any(|c| c.is_ascii_alphabetic())
}

/// The source of a request: the peer address, or the address a trusted
/// proxy saw.
///
/// The rule is the module doc comment's. `peer` is `None` when the API
/// was not served with connect information; the source is then
/// [`UNKNOWN_SOURCE`], and no header is read.
#[must_use]
pub fn resolve(
    peer: Option<SocketAddr>,
    forwarded_for: Option<&str>,
    trusted: &TrustedProxies,
) -> String {
    let Some(peer) = peer else {
        return UNKNOWN_SOURCE.to_owned();
    };
    let peer_ip = crate::internal_hosts::canonical(&peer.ip());
    // A peer nobody vouched for IS the source, and its header is never
    // read. An empty trusted list vouches for nobody, so with it the
    // header is never trusted either (fail closed).
    if !trusted.is_trusted(&peer_ip) {
        return peer_ip.to_string();
    }
    match forwarded_for.and_then(|header| rightmost_untrusted(header, trusted)) {
        Some(source) => source.to_string(),
        None => peer_ip.to_string(),
    }
}

/// The right-most entry of `X-Forwarded-For` that is not itself trusted.
///
/// `None` when the header cannot be used at all: an entry that is not an
/// address makes the whole header unusable, because the entries to its
/// left cannot be trusted either — a header that mixes an address with
/// nonsense is a header nobody can read.
fn rightmost_untrusted(header: &str, trusted: &TrustedProxies) -> Option<IpAddr> {
    let mut candidate: Option<IpAddr> = None;
    for entry in header.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let ip = parse_forwarded_entry(entry)?;
        if !trusted.is_trusted(&ip) {
            candidate = Some(ip);
        }
    }
    candidate
}

/// The address one `X-Forwarded-For` entry names.
///
/// An entry is an address (`192.0.2.1`), an address with a port
/// (`192.0.2.1:443`), or a bracketed address with a port (`[2001:db8::1]:443`).
/// `None` when it is none of those.
fn parse_forwarded_entry(entry: &str) -> Option<IpAddr> {
    if let Ok(ip) = entry.parse::<IpAddr>() {
        return Some(crate::internal_hosts::canonical(&ip));
    }
    if let Ok(address) = entry.parse::<SocketAddr>() {
        return Some(crate::internal_hosts::canonical(&address.ip()));
    }
    let bracketed = entry.strip_prefix('[')?;
    let (inside, _) = bracketed.split_once(']')?;
    let ip = inside.parse::<IpAddr>().ok()?;
    Some(crate::internal_hosts::canonical(&ip))
}

/// The source of a request, resolved once per request.
#[derive(Debug, Clone)]
pub struct ClientSource(String);

impl ClientSource {
    /// A source that is already known.
    ///
    /// The extractor builds one from the request; this is for a caller
    /// that already holds the source — a test that calls a handler
    /// directly, or a route that resolves it itself.
    #[must_use]
    pub fn new(source: impl Into<String>) -> Self {
        Self(source.into())
    }

    /// The source: an address, or [`UNKNOWN_SOURCE`].
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromRequestParts<AppState> for ClientSource {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|connect_info| connect_info.0);
        let forwarded_for = parts
            .headers
            .get(FORWARDED_FOR)
            .and_then(|value| value.to_str().ok());
        let trusted = state.trusted_proxies.read().await;
        Ok(Self(resolve(peer, forwarded_for, &trusted)))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn addr(raw: &str) -> SocketAddr {
        raw.parse().unwrap()
    }

    fn trusted(raw: &str) -> TrustedProxies {
        let (networks, service_names) = parse_trusted_proxies(raw).unwrap();
        TrustedProxies::new(networks, service_names)
    }

    #[test]
    fn a_forged_header_from_an_untrusted_peer_is_ignored() {
        let proxies = trusted("192.0.2.0/24");
        let source = resolve(
            Some(addr("198.51.100.9:5555")),
            Some("203.0.113.7"),
            &proxies,
        );
        assert_eq!(source, "198.51.100.9");
    }

    #[test]
    fn a_trusted_peer_takes_the_rightmost_untrusted_entry() {
        let proxies = trusted("192.0.2.0/24");
        let source = resolve(
            Some(addr("192.0.2.10:5555")),
            Some("203.0.113.7, 198.51.100.4"),
            &proxies,
        );
        assert_eq!(source, "198.51.100.4");
    }

    #[test]
    fn a_chain_of_only_trusted_entries_falls_back_to_the_peer() {
        let proxies = trusted("192.0.2.0/24");
        let source = resolve(
            Some(addr("192.0.2.10:5555")),
            Some("192.0.2.11, 192.0.2.12"),
            &proxies,
        );
        assert_eq!(source, "192.0.2.10");
    }

    #[test]
    fn ipv4_and_ipv6_are_both_sources() {
        let proxies = trusted("192.0.2.0/24, 2001:db8::/32");
        assert_eq!(
            resolve(Some(addr("192.0.2.10:1")), Some("203.0.113.7"), &proxies),
            "203.0.113.7"
        );
        // The source is OUTSIDE the trusted network: an entry inside it is
        // itself trusted, and a trusted entry is skipped (the proxy wrote
        // it, the caller did not).
        assert_eq!(
            resolve(
                Some(addr("[2001:db8::10]:1")),
                Some("2001:db9::7"),
                &proxies
            ),
            "2001:db9::7"
        );
        assert_eq!(
            resolve(
                Some(addr("[2001:db8::10]:1")),
                Some("[2001:db9::7]:443"),
                &proxies
            ),
            "2001:db9::7"
        );
    }

    #[test]
    fn a_malformed_header_falls_back_to_the_peer() {
        let proxies = trusted("192.0.2.0/24");
        let source = resolve(
            Some(addr("192.0.2.10:5555")),
            Some("203.0.113.7, not-an-address"),
            &proxies,
        );
        assert_eq!(source, "192.0.2.10");
    }

    #[test]
    fn an_empty_trusted_list_never_trusts_a_header() {
        let proxies = trusted("");
        let source = resolve(
            Some(addr("198.51.100.9:5555")),
            Some("203.0.113.7"),
            &proxies,
        );
        assert_eq!(source, "198.51.100.9");
    }

    #[test]
    fn a_request_without_a_peer_is_unknown() {
        let proxies = trusted("192.0.2.0/24");
        // The value is spelled out, not compared with the constant: a test
        // that compares the answer with the constant it is about proves
        // nothing about what a caller sees.
        assert_eq!(resolve(None, Some("203.0.113.7"), &proxies), "unknown");
        assert_eq!(resolve(None, None, &proxies), "unknown");
        // One fixed bucket for every such caller, never an empty string:
        // an empty source would put every request in a bucket of its own
        // name, and the per-source limit would count nothing.
        assert_eq!(UNKNOWN_SOURCE, "unknown");
    }

    #[test]
    fn a_trusted_peer_without_a_header_falls_back_to_the_peer() {
        let proxies = trusted("192.0.2.0/24");
        assert_eq!(
            resolve(Some(addr("192.0.2.10:1")), None, &proxies),
            "192.0.2.10"
        );
    }

    #[test]
    fn the_list_splits_networks_from_service_names() {
        let (networks, names) =
            parse_trusted_proxies("192.0.2.0/24, 198.51.100.5, console, lakehouse-frontend")
                .unwrap();
        assert_eq!(networks.len(), 2);
        assert_eq!(
            names,
            vec!["console".to_owned(), "lakehouse-frontend".to_owned()]
        );
    }

    #[test]
    fn a_list_holding_nonsense_is_refused() {
        assert!(parse_trusted_proxies("192.0.2.0/24, not a name!").is_err());
        // A network written with host bits set is refused the same way
        // the connector allowlist refuses it.
        assert!(parse_trusted_proxies("192.0.2.5/24").is_err());
    }
}
