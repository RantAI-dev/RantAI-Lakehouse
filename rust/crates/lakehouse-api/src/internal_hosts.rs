//! Which private/internal addresses a connector dial may reach.
//!
//! `connector_probe::is_blocked_ip` refuses every private, loopback,
//! link-local and unspecified address by default (the SSRF guard). Two
//! operator switches lift that:
//!
//! - `CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS=true` lifts it for every
//!   internal address ([`InternalHosts::allow_all`]).
//! - `CONNECTOR_PROBE_ALLOWED_CIDRS` lifts it only for the listed networks
//!   ([`InternalHosts::allowed`]), e.g. the LAN a customer's databases live
//!   on, while the compose network and the host itself stay refused.
//!
//! The list never reaches loopback, link-local (`169.254/16`, where cloud
//! instance metadata lives, and `fe80::/10`), multicast or the unspecified
//! address: listing `0.0.0.0/0` opens private ranges, not those. Only
//! `allow_all` does.
//!
//! Mirrors `dagster/dispar_orchestrate/ssrf_guard.py`'s
//! `INGEST_ALLOWED_CIDRS`, the Dagster-side counterpart, configured
//! separately for the same reason `INGEST_ALLOW_INTERNAL_HOSTS` is.

use std::net::IpAddr;

use ipnet::IpNet;

/// The internal addresses a dial may reach despite the SSRF guard.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InternalHosts {
    /// Every internal address is allowed (`CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS`).
    pub allow_all: bool,
    /// Private networks allowed even when `allow_all` is false
    /// (`CONNECTOR_PROBE_ALLOWED_CIDRS`).
    pub allowed: Vec<IpNet>,
}

/// Fixed policies for tests; production code always builds one from
/// config (`Config::connector_internal_hosts`).
#[cfg(test)]
impl InternalHosts {
    /// Every internal address allowed.
    pub const ALL: Self = Self {
        allow_all: true,
        allowed: Vec::new(),
    };

    /// No internal address allowed: the guard's default.
    pub const NONE: Self = Self {
        allow_all: false,
        allowed: Vec::new(),
    };
}

impl InternalHosts {
    /// Whether `ip`, an address the SSRF guard refuses by default, may be
    /// dialed anyway.
    #[must_use]
    pub fn permits(&self, ip: &IpAddr) -> bool {
        if self.allow_all {
            return true;
        }
        let ip = canonical(ip);
        if never_listed(&ip) {
            return false;
        }
        self.allowed.iter().any(|net| net.contains(&ip))
    }
}

/// An IPv4-mapped IPv6 address as the IPv4 address it wraps, so
/// `::ffff:192.168.18.5` matches `192.168.18.0/24`.
fn canonical(ip: &IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(*ip, IpAddr::V4),
        IpAddr::V4(_) => *ip,
    }
}

/// Addresses the allowlist can never open (see the module doc).
fn never_listed(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback() || v4.is_link_local() || v4.is_unspecified() || v4.is_multicast()
        }
        IpAddr::V6(v6) => {
            let is_link_local = v6.segments()[0] & 0xffc0 == 0xfe80; // fe80::/10
            v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() || is_link_local
        }
    }
}

/// Parse an allowlist: networks (`192.168.18.0/24`) or single addresses
/// (`192.168.18.205`), separated by commas or whitespace. Empty input is
/// an empty list.
///
/// # Errors
///
/// Returns a message naming the first entry that is neither, or a network
/// written with host bits set (`192.168.18.5/24`): that usually means one
/// host was meant, and quietly widening it to the whole `/24` would allow
/// more than was asked for.
pub fn parse_cidrs(raw: &str) -> Result<Vec<IpNet>, String> {
    raw.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            if let Ok(net) = entry.parse::<IpNet>() {
                if net.trunc() != net {
                    return Err(format!(
                        "{entry:?} has host bits set: write the network as {} or the single \
                         address without a prefix",
                        net.trunc()
                    ));
                }
                return Ok(net);
            }
            entry.parse::<IpAddr>().map(IpNet::from).map_err(|_| {
                format!("{entry:?} is not a network (e.g. 192.168.18.0/24) or an address")
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn listing(raw: &str) -> InternalHosts {
        InternalHosts {
            allow_all: false,
            allowed: parse_cidrs(raw).unwrap(),
        }
    }

    fn ip(raw: &str) -> IpAddr {
        raw.parse().unwrap()
    }

    #[test]
    fn a_listed_network_opens_only_its_own_addresses() {
        let hosts = listing("192.168.18.0/24");
        assert!(hosts.permits(&ip("192.168.18.205")));
        assert!(!hosts.permits(&ip("192.168.19.5")));
        assert!(!hosts.permits(&ip("172.18.0.4")));
    }

    #[test]
    fn a_bare_address_is_one_host() {
        let hosts = listing("192.168.18.205");
        assert!(hosts.permits(&ip("192.168.18.205")));
        assert!(!hosts.permits(&ip("192.168.18.202")));
    }

    #[test]
    fn an_ipv4_mapped_ipv6_address_matches_its_ipv4_network() {
        assert!(listing("192.168.18.0/24").permits(&ip("::ffff:192.168.18.205")));
    }

    #[test]
    fn loopback_link_local_and_unspecified_are_never_opened_by_the_list() {
        let hosts = listing("0.0.0.0/0, ::/0");
        assert!(hosts.permits(&ip("10.0.0.1")));
        for blocked in [
            "127.0.0.1",
            "169.254.169.254",
            "0.0.0.0",
            "::1",
            "fe80::1",
            "::",
        ] {
            assert!(!hosts.permits(&ip(blocked)), "{blocked} must stay refused");
        }
    }

    #[test]
    fn allow_all_opens_everything_and_none_opens_nothing() {
        assert!(InternalHosts::ALL.permits(&ip("127.0.0.1")));
        assert!(!InternalHosts::NONE.permits(&ip("192.168.18.205")));
    }

    #[test]
    fn parse_accepts_commas_and_whitespace_and_ignores_blanks() {
        let nets = parse_cidrs(" 10.0.0.0/8,192.168.18.205  fd00::/8 ,").unwrap();
        assert_eq!(nets.len(), 3);
        assert!(parse_cidrs("").unwrap().is_empty());
    }

    #[test]
    fn parse_refuses_garbage_and_host_bits() {
        assert!(parse_cidrs("192.168.18.0/33").is_err());
        assert!(parse_cidrs("lan").is_err());
        let err = parse_cidrs("192.168.18.5/24").unwrap_err();
        assert!(err.contains("192.168.18.0/24"), "{err}");
    }
}
