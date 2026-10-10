//! The address policy for alert webhooks (`SEC-10`): the API crate's side of
//! `lakehouse_notify::WebhookSender`.
//!
//! # Why this is here and not in `lakehouse-notify`
//!
//! "Which addresses are internal" is one decision, owned by
//! `connector_probe::is_blocked_ip` (widened by `SEC-15`) and
//! [`crate::internal_hosts::InternalHosts`]. Both live in this crate, and
//! `lakehouse-notify` is below it, so the sender takes a
//! `TargetResolver` and this module implements it by calling that same
//! policy through `connector_probe::first_refused`. There is no second copy
//! of the blocked set to drift from the first.
//!
//! # What is allowed
//!
//! Everything the connector probe allows by default, nothing it refuses,
//! plus the networks an administrator lists in `WEBHOOK_ALLOWED_CIDRS`
//! (`Config::webhook_internal_hosts`). That setting is separate from the
//! connector one and has no "allow all" form; the list still cannot open
//! loopback, link-local (the cloud metadata address), multicast or `0/8`.
//!
//! # What the log gets, what the caller gets
//!
//! A refusal is logged with the host and the address it resolved to, for the
//! operator; the caller gets `lakehouse_notify::WebhookError`'s fixed
//! message, which names neither (a resolved internal address is itself
//! information about the network).

use std::net::SocketAddr;
use std::sync::Arc;

use lakehouse_notify::{ResolveFuture, TargetRefusal, TargetResolver, WebhookSender};

use crate::config::Config;
use crate::connector_probe::first_refused;
use crate::internal_hosts::InternalHosts;

/// Resolves a webhook host and refuses it unless every address passes the
/// shared address policy.
#[derive(Debug, Clone)]
pub struct WebhookGuard {
    hosts: InternalHosts,
    /// Test seam: names that resolve to fixed addresses instead of through
    /// the system resolver, so a test can have a "public" name resolve to
    /// loopback without a DNS query.
    #[cfg(test)]
    names: std::collections::HashMap<String, Vec<std::net::IpAddr>>,
}

impl WebhookGuard {
    /// A guard that allows the internal addresses `hosts` lists.
    #[must_use]
    pub fn new(hosts: InternalHosts) -> Self {
        Self {
            hosts,
            #[cfg(test)]
            names: std::collections::HashMap::new(),
        }
    }

    async fn lookup(&self, host: &str, port: u16) -> Vec<SocketAddr> {
        #[cfg(test)]
        if let Some(ips) = self.names.get(host) {
            return ips.iter().map(|ip| SocketAddr::new(*ip, port)).collect();
        }
        match tokio::net::lookup_host((host, port)).await {
            Ok(iter) => iter.collect(),
            Err(err) => {
                // The resolver's own text is for the log only.
                tracing::warn!(%host, error = %err, "SEC-10: webhook host did not resolve");
                Vec::new()
            }
        }
    }
}

impl TargetResolver for WebhookGuard {
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> ResolveFuture<'a> {
        Box::pin(async move {
            let addrs = self.lookup(host, port).await;
            if addrs.is_empty() {
                return Err(TargetRefusal::Unresolved);
            }
            if let Some(refused) = first_refused(&addrs, &self.hosts) {
                tracing::warn!(
                    %host,
                    resolved = %refused,
                    "SEC-10: refused a webhook target that resolves to an internal address"
                );
                return Err(TargetRefusal::NotAllowed);
            }
            Ok(addrs)
        })
    }
}

/// The webhook sender every alert path uses: the one place a `Config`
/// becomes an address policy for outbound alert calls.
#[must_use]
pub fn sender(config: &Config) -> WebhookSender {
    WebhookSender::new(Arc::new(WebhookGuard::new(config.webhook_internal_hosts())))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::net::IpAddr;

    use lakehouse_notify::{DeliverResult, WebhookError};
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn guard(allowed: &str) -> WebhookGuard {
        WebhookGuard::new(InternalHosts {
            allow_all: false,
            allowed: crate::internal_hosts::parse_cidrs(allowed).unwrap(),
        })
    }

    fn named(mut guard: WebhookGuard, name: &str, ips: &[&str]) -> WebhookGuard {
        let ips: Vec<IpAddr> = ips.iter().map(|ip| ip.parse().unwrap()).collect();
        guard.names.insert(name.to_owned(), ips);
        guard
    }

    fn sender_for(guard: WebhookGuard) -> WebhookSender {
        WebhookSender::new(Arc::new(guard))
    }

    async fn listener() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        server
    }

    /// `SEC-10-AC1`: every refused form is refused with the fixed message
    /// BEFORE any connection: the listener on loopback sees nothing, even
    /// for the forms that point at it.
    #[tokio::test]
    async fn blocked_addresses_are_refused_before_any_connection() {
        let server = listener().await;
        let port = server.address().port();
        let sender = sender_for(named(
            named(guard(""), "inward.test", &["10.1.2.3"]),
            "wrapped.test",
            &["::ffff:127.0.0.1"],
        ));
        let urls = [
            format!("http://127.0.0.1:{port}/x"),
            format!("http://[::1]:{port}/x"),
            "http://10.1.2.3/x".to_owned(),
            "http://169.254.169.254/latest/meta-data".to_owned(),
            "http://100.64.0.1/x".to_owned(),
            format!("http://[::ffff:127.0.0.1]:{port}/x"),
            // Numeric spellings the URL parser normalises to 127.0.0.1.
            format!("http://2130706433:{port}/x"),
            format!("http://0x7f.1:{port}/x"),
            "http://inward.test/x".to_owned(),
            "http://wrapped.test/x".to_owned(),
        ];
        for url in &urls {
            assert_eq!(
                sender.send(url, "t", "x").await,
                DeliverResult {
                    ok: false,
                    error: Some("webhook address is not allowed".to_owned())
                },
                "{url}"
            );
            assert_eq!(
                sender.check(url).await,
                Err(WebhookError::NotAllowed),
                "{url}"
            );
        }
        assert!(
            server.received_requests().await.unwrap().is_empty(),
            "a refused target must never be connected to"
        );
    }

    /// A name with one public and one internal answer is refused as a whole
    /// (the connector probe's rule): connecting to the public one first
    /// would still let a retry land on the internal one.
    #[tokio::test]
    async fn a_name_with_any_internal_answer_is_refused() {
        let sender = sender_for(named(guard(""), "mixed.test", &["203.0.113.7", "10.0.0.5"]));
        assert_eq!(
            sender.check("https://mixed.test/hook").await,
            Err(WebhookError::NotAllowed)
        );
    }

    /// A public-looking name is delivered, and the connection goes to the
    /// checked address. The name's only answer is the test listener's
    /// loopback address, which the guard is told to allow for this test
    /// alone (`allow_all`; production has no such switch for webhooks), and
    /// the name does not exist in DNS (`.test`), so delivery proves the
    /// request was pinned to the checked address.
    #[tokio::test]
    async fn a_name_resolving_to_a_checked_address_is_delivered_there() {
        let server = listener().await;
        let port = server.address().port();
        let mut open = guard("");
        open.hosts.allow_all = true;
        let sender = sender_for(named(open, "hooks.test", &["127.0.0.1"]));
        let url = format!("http://hooks.test:{port}/x");
        assert_eq!(
            sender.send(&url, "t", "x").await,
            DeliverResult {
                ok: true,
                error: None
            }
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    /// `SEC-10`: the allowlist opens exactly what it lists: a listed private
    /// address passes the check, its neighbour does not, and listing
    /// loopback or the metadata address changes nothing.
    #[tokio::test]
    async fn the_allowlist_opens_exactly_the_listed_hosts() {
        let sender = sender_for(guard("192.168.18.205, 127.0.0.0/8, 169.254.169.254"));
        assert_eq!(sender.check("http://192.168.18.205:8080/x").await, Ok(()));
        for refused in [
            "http://192.168.18.206/x",
            "http://127.0.0.1/x",
            "http://169.254.169.254/x",
        ] {
            assert_eq!(
                sender.check(refused).await,
                Err(WebhookError::NotAllowed),
                "{refused}"
            );
        }
    }

    /// `SEC-10-AC2`: a public endpoint that answers 302 to an internal
    /// address is a failed delivery, and the internal address receives
    /// nothing.
    #[tokio::test]
    async fn a_redirect_to_an_internal_address_is_not_followed() {
        let inward = listener().await;
        let public = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(302).insert_header("Location", inward.uri().as_str()),
            )
            .mount(&public)
            .await;
        let mut open = guard("");
        open.hosts.allow_all = true;
        let sender = sender_for(named(open, "public.test", &["127.0.0.1"]));
        let url = format!("http://public.test:{}/x", public.address().port());
        assert_eq!(
            sender.send(&url, "t", "x").await,
            DeliverResult {
                ok: false,
                error: Some("webhook redirect not followed".to_owned())
            }
        );
        assert!(inward.received_requests().await.unwrap().is_empty());
    }

    /// `SEC-10`: the shipped configuration path. With no setting, loopback
    /// is refused through `sender(config)`; the test-only switch is off
    /// unless the test asks for it.
    #[tokio::test]
    async fn the_configured_sender_refuses_loopback_by_default() {
        let config = Config::from_map(&std::collections::HashMap::new()).unwrap();
        assert!(!config.webhook_internal_hosts().allow_all);
        assert_eq!(
            sender(&config).check("http://127.0.0.1:9/x").await,
            Err(WebhookError::NotAllowed)
        );
    }
}
