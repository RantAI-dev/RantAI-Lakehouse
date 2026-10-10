//! The webhook sender (`SEC-10`): the only code in this workspace that makes
//! an alert or digest call a URL a user typed.
//!
//! # Why it is built this way
//!
//! Before `SEC-10` the sender checked that a URL started with `http(s)://`
//! and handed it to a default `reqwest` client, so a user who may create
//! alerts could make the server call anything it can reach (a service in the
//! customer's network, the cloud metadata address, the server itself), and a
//! public URL could bounce the call inward with a redirect. Three rules close
//! that, and all three live here so no caller can skip one:
//!
//! 1. **The address is checked, then used.** The host is resolved once by a
//!    [`TargetResolver`], which refuses a blocked address, and the request is
//!    pinned (`ClientBuilder::resolve_to_addrs`) to the addresses it approved.
//!    The HTTP client never resolves the name itself, so the answer cannot
//!    change between the check and the connection (a DNS rebind). For an
//!    `https` URL the TLS server name and certificate check still use the
//!    URL's host name: only the address lookup is overridden.
//! 2. **Redirects are not followed.** A redirect is a second target nobody
//!    checked, so `3xx` is a failed delivery.
//! 3. **No library text leaves this module.** A failure is one of the fixed
//!    messages of [`WebhookError`]; `reqwest`'s own error text (which can
//!    carry the URL, including a token in its path) is classified, never
//!    stored or returned.
//!
//! # Where the address policy lives
//!
//! The policy (which addresses are blocked, which an administrator allowed)
//! is the connector probe's `is_blocked_ip` plus `InternalHosts`, in the API
//! crate, which this crate cannot depend on. [`TargetResolver`] is the seam:
//! the API crate implements it with that one policy and passes it in, so
//! there is one implementation of the blocked set (`SEC-10` plan, T1).

#[cfg(any(test, feature = "test-support"))]
use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use reqwest::Url;
use serde_json::json;

use crate::DeliverResult;

/// How long a whole delivery may take. Before `SEC-10` the client had no
/// timeout at all (`reqwest::Client::new()`), so one unresponsive webhook
/// could hold an alert run open indefinitely.
const TOTAL_TIMEOUT: Duration = Duration::from_secs(10);

/// How long the TCP/TLS connection may take to establish.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Why a resolver refused a target. Deliberately carries no text: what the
/// resolver logged is for the operator, what the caller sees is
/// [`WebhookError`]'s fixed message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetRefusal {
    /// At least one resolved address is blocked and not allowed.
    NotAllowed,
    /// The name did not resolve to any address.
    Unresolved,
}

/// The boxed future a [`TargetResolver`] returns (kept manual so this crate
/// needs no `async-trait`).
pub type ResolveFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<SocketAddr>, TargetRefusal>> + Send + 'a>>;

/// Resolves a webhook host and decides whether every address it gets may be
/// called. An `Ok` is the proof that the check ran: the sender connects only
/// to the returned addresses.
pub trait TargetResolver: Send + Sync {
    /// Resolve `host` (an IP literal without brackets, or a domain name) and
    /// refuse it if any resolved address is not allowed.
    ///
    /// # Errors
    ///
    /// [`TargetRefusal::NotAllowed`] when any address is blocked and not
    /// allowed, [`TargetRefusal::Unresolved`] when there is none.
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> ResolveFuture<'a>;
}

/// Why a webhook was not delivered or may not be saved. Every variant has a
/// fixed message of ours ([`fmt::Display`]); none carries upstream text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebhookError {
    /// Not an `http(s)` URL, or it has user info or no host.
    InvalidUrl,
    /// The host resolves to an address webhooks may not reach.
    NotAllowed,
    /// The host name did not resolve.
    Unresolved,
    /// The endpoint answered with a redirect, which is not followed.
    Redirect,
    /// The endpoint answered with a non-success status.
    Status(u16),
    /// The request exceeded the time limit.
    TimedOut,
    /// The connection could not be made.
    ConnectFailed,
    /// Any other failure of the request.
    Failed,
}

impl fmt::Display for WebhookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl => f.write_str("invalid webhook URL"),
            Self::NotAllowed => f.write_str("webhook address is not allowed"),
            Self::Unresolved => f.write_str("webhook host could not be resolved"),
            Self::Redirect => f.write_str("webhook redirect not followed"),
            Self::Status(code) => write!(f, "webhook HTTP {code}"),
            Self::TimedOut => f.write_str("webhook request timed out"),
            Self::ConnectFailed => f.write_str("webhook connection failed"),
            Self::Failed => f.write_str("webhook request failed"),
        }
    }
}

impl From<TargetRefusal> for WebhookError {
    fn from(refusal: TargetRefusal) -> Self {
        match refusal {
            TargetRefusal::NotAllowed => Self::NotAllowed,
            TargetRefusal::Unresolved => Self::Unresolved,
        }
    }
}

/// A webhook URL that passed the check, with the addresses to pin the
/// connection to (none for an IP literal, which needs no lookup).
struct Checked {
    url: Url,
    pin: Option<(String, Vec<SocketAddr>)>,
}

/// Sends webhooks through one [`TargetResolver`].
#[derive(Clone)]
pub struct WebhookSender {
    resolver: Arc<dyn TargetResolver>,
}

impl fmt::Debug for WebhookSender {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebhookSender").finish_non_exhaustive()
    }
}

impl WebhookSender {
    /// A sender whose every target goes through `resolver`.
    #[must_use]
    pub fn new(resolver: Arc<dyn TargetResolver>) -> Self {
        Self { resolver }
    }

    /// Check a URL without sending: the scheme, user info, host, and the
    /// resolver's verdict on every address it resolves to. Used when a rule
    /// is saved; a name can resolve differently later, so [`Self::send`]
    /// checks again.
    ///
    /// # Errors
    ///
    /// [`WebhookError::InvalidUrl`], [`WebhookError::NotAllowed`] or
    /// [`WebhookError::Unresolved`].
    pub async fn check(&self, raw: &str) -> Result<(), WebhookError> {
        self.prepare(raw).await.map(|_| ())
    }

    async fn prepare(&self, raw: &str) -> Result<Checked, WebhookError> {
        let url = Url::parse(raw).map_err(|_| WebhookError::InvalidUrl)?;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(WebhookError::InvalidUrl);
        }
        let host = url
            .host_str()
            .filter(|host| !host.is_empty())
            .ok_or(WebhookError::InvalidUrl)?
            .to_owned();
        let port = url
            .port_or_known_default()
            .ok_or(WebhookError::InvalidUrl)?;
        // `host_str` keeps the brackets of an IPv6 literal; resolvers and
        // `IpAddr` parsing want them off. The URL parser has already
        // normalised numeric forms (`2130706433`, `0x7f.1`) to dotted
        // quads, so a literal here is what the connection would dial.
        let bare = host.trim_start_matches('[').trim_end_matches(']');
        let is_literal = bare.parse::<IpAddr>().is_ok();
        let addrs = self.resolver.resolve(bare, port).await?;
        if addrs.is_empty() {
            return Err(WebhookError::Unresolved);
        }
        let pin = (!is_literal).then_some((host, addrs));
        Ok(Checked { url, pin })
    }

    /// `POST` the alert as an incoming webhook (the same fan-out body as the
    /// `TypeScript`'s `sendWebhook`: `text` for Slack, `content` for Discord,
    /// `title`/`message` for a generic endpoint).
    ///
    /// Never returns `Err`; every failure is a [`DeliverResult`] whose text
    /// is one of [`WebhookError`]'s fixed messages.
    pub async fn send(&self, raw_url: &str, title: &str, text: &str) -> DeliverResult {
        match self.try_send(raw_url, title, text).await {
            Ok(()) => DeliverResult::ok(),
            Err(err) => DeliverResult::err(err.to_string()),
        }
    }

    async fn try_send(&self, raw_url: &str, title: &str, text: &str) -> Result<(), WebhookError> {
        let checked = self.prepare(raw_url).await?;
        let mut builder = reqwest::Client::builder()
            // A redirect is a second target nobody checked.
            .redirect(reqwest::redirect::Policy::none())
            // A proxy resolves the name itself, which defeats the pin.
            .no_proxy()
            .timeout(TOTAL_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT);
        if let Some((host, addrs)) = &checked.pin {
            builder = builder.resolve_to_addrs(host, addrs);
        }
        let client = builder.build().map_err(|_| WebhookError::Failed)?;
        let body = json!({
            "text": format!("*{title}*\n{text}"),
            "content": format!("**{title}**\n{text}"),
            "title": title,
            "message": text,
        });
        let response = client
            .post(checked.url)
            .json(&body)
            .send()
            .await
            .map_err(|err| {
                if err.is_timeout() {
                    WebhookError::TimedOut
                } else if err.is_connect() {
                    WebhookError::ConnectFailed
                } else {
                    WebhookError::Failed
                }
            })?;
        let status = response.status();
        if status.is_success() {
            Ok(())
        } else if status.is_redirection() {
            Err(WebhookError::Redirect)
        } else {
            Err(WebhookError::Status(status.as_u16()))
        }
    }
}

/// A resolver for tests that approves everything it resolves and can map a
/// name to a fixed address, so a test can reach a local listener through a
/// "public" name without DNS. Not a policy; anything that needs a refusal
/// tests the real resolver in the API crate. Compiled only for this crate's
/// tests and under the `test-support` feature (which only `[dev-dependencies]`
/// enable), so it cannot be linked into a service build.
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Default, Clone)]
pub struct MappedResolver {
    names: HashMap<String, SocketAddr>,
}

#[cfg(any(test, feature = "test-support"))]
impl MappedResolver {
    /// A resolver that approves IP literals as they are and nothing else.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Map `name` to `addr` (the port the request asks for is ignored in
    /// favour of `addr`'s, as a listener on an ephemeral port needs).
    #[must_use]
    pub fn with(mut self, name: &str, addr: SocketAddr) -> Self {
        self.names.insert(name.to_owned(), addr);
        self
    }

    /// Wrap into a sender.
    #[must_use]
    pub fn into_sender(self) -> WebhookSender {
        WebhookSender::new(Arc::new(self))
    }
}

#[cfg(any(test, feature = "test-support"))]
impl TargetResolver for MappedResolver {
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> ResolveFuture<'a> {
        Box::pin(async move {
            if let Some(addr) = self.names.get(host) {
                return Ok(vec![*addr]);
            }
            match host.parse::<IpAddr>() {
                Ok(ip) => Ok(vec![SocketAddr::new(ip, port)]),
                Err(_) => Err(TargetRefusal::Unresolved),
            }
        })
    }
}
