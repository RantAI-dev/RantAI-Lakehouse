//! Notification delivery (alerts & digests) to two channels, porting
//! `src/services/clients/notify.ts`:
//!
//! - **webhook**: `POST` JSON to an incoming-webhook URL (Slack/Discord/
//!   generic). The URL is supplied by the caller/user at call time — this
//!   crate never stores one, matching the TypeScript's "self-contained, no
//!   secret in the server" design. The address check, the pinned
//!   connection and the no-redirect rule are in [`WebhookSender`]
//!   (`SEC-10`).
//! - **email**: via `SMTP`, using `lettre`. Only active when a host is
//!   configured; the server never hardcodes a password in code or a
//!   database.

mod webhook;

#[cfg(any(test, feature = "test-support"))]
pub use webhook::MappedResolver;
pub use webhook::{ResolveFuture, TargetRefusal, TargetResolver, WebhookError, WebhookSender};

use lettre::message::{Mailbox, SinglePart, header};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use serde::Serialize;

/// Why a delivery did not happen, as a fixed set of reasons the product
/// words itself (`SRC-17` F1, delivery review 2026-10-12; principles 2 and 4).
///
/// The text of a failure never comes from the other side: not a webhook's
/// status line or body, not an `SMTP` reply, not a driver's error (which can
/// carry a host name or an address). Each upstream failure is classified into
/// one of these before it is returned, stored on an alert or shown. The
/// [`Self::message`] strings are what the Alerts page prints; a test pins
/// every one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryFailure {
    /// The webhook URL is not an `http(s)` address.
    WebhookAddressRefused,
    /// The webhook server could not be reached (connect, `DNS`, timeout, TLS).
    WebhookUnreachable,
    /// The webhook server answered with a non-success status.
    WebhookServerError,
    /// The webhook server answered with a redirect, which is never followed
    /// (`SEC-10`: a redirect is a second target nobody checked).
    WebhookRedirected,
    /// `SMTP_HOST` is not set.
    EmailNotConfigured,
    /// The e-mail address (or the message built from it) is not valid.
    EmailAddressRefused,
    /// The mail server could not be reached.
    MailServerUnreachable,
    /// The mail server answered and refused the message.
    MailServerRefused,
    /// The rule names a channel this build does not send to.
    ChannelUnsupported,
}

impl DeliveryFailure {
    /// The fixed sentence for this reason; nothing from the failed call is
    /// ever added to it.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::WebhookAddressRefused => "the webhook address was refused",
            Self::WebhookUnreachable => "the webhook server could not be reached",
            Self::WebhookServerError => "the webhook server answered with an error",
            Self::WebhookRedirected => "the webhook server redirected the request",
            Self::EmailNotConfigured => "e-mail is not set up on this installation",
            Self::EmailAddressRefused => "the e-mail address was refused",
            Self::MailServerUnreachable => "the mail server could not be reached",
            Self::MailServerRefused => "the mail server refused the message",
            Self::ChannelUnsupported => "the rule's channel is not supported",
        }
    }
}

/// Outcome of a delivery attempt, matching the TypeScript's
/// `DeliverResult`. Deliberately not a `Result<(), String>`: the
/// TypeScript never throws from `sendWebhook`/`sendEmail`/`deliver` — every
/// failure mode (bad URL, unconfigured `SMTP`, invalid address, network
/// error) is reported through this struct instead.
///
/// `error`, when present, is always the [`DeliveryFailure::message`] of a
/// fixed reason (`SRC-17` F1): this struct is serialized into the
/// `/api/alerts/run` response and stored on alert instances, so it must not
/// carry upstream text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeliverResult {
    /// Whether delivery succeeded.
    pub ok: bool,
    /// A human-readable failure reason, present when `ok` is `false`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl DeliverResult {
    fn ok() -> Self {
        Self {
            ok: true,
            error: None,
        }
    }

    fn err(failure: DeliveryFailure) -> Self {
        Self {
            ok: false,
            error: Some(failure.message().to_owned()),
        }
    }
}

/// `SMTP` configuration needed to send email, matching the fields read
/// from `process.env.SMTP_*` in the TypeScript's `sendEmail`.
///
/// `EmailSender::send` folds in `notify.ts`'s exact validation and
/// fallback order — see that method's doc comment.
#[derive(Debug, Clone)]
pub struct SmtpConfig {
    /// `SMTP_HOST`. `None` means `SMTP` is not configured at all.
    pub host: Option<String>,
    /// `SMTP_PORT`, already resolved by the caller (default `587`).
    pub port: u16,
    /// The *effective* implicit-TLS decision — `Config::smtp_secure`
    /// already folds in `SMTP_SECURE === "true" || port === 465`, so this
    /// is consumed directly rather than recomputed here.
    pub secure: bool,
    /// `SMTP_USER`. `None` means no `SMTP` auth is sent at all.
    pub user: Option<String>,
    /// `SMTP_PASS`, default `""`.
    pub pass: String,
    /// `SMTP_FROM` (already resolved with its `??` fallback chain by the
    /// caller).
    pub from: String,
}

/// Build the `SMTP` message that `EmailSender::send` would send, without
/// sending it — split out so message construction (headers, from/to,
/// body) can be tested without a live `SMTP` connection.
///
/// # Errors
///
/// Returns `Err` if `from`/`to` don't parse as mailboxes or `Message`
/// construction otherwise fails (e.g. malformed header values).
fn build_message(
    from: &str,
    to: &str,
    subject: &str,
    html: &str,
) -> Result<Message, lettre::error::Error> {
    Message::builder()
        .from(from.parse().unwrap_or_else(|_| {
            // Falls back to a syntactically valid placeholder rather than
            // erroring the whole send on a malformed `SMTP_FROM` — mirrors
            // how permissive the TypeScript config resolution is about
            // this field (never validated before use). The literal below
            // is a valid address by construction, so this never panics.
            Mailbox::new(
                None,
                "rantai-lake@localhost"
                    .parse()
                    .unwrap_or_else(|_| unreachable!("literal address is always valid")),
            )
        }))
        .to(to.parse().map_err(|_| lettre::error::Error::MissingTo)?)
        .subject(subject)
        .singlepart(
            SinglePart::builder()
                .header(header::ContentType::TEXT_HTML)
                .body(html.to_owned()),
        )
}

/// Sends email via `SMTP`, porting `sendEmail`.
pub struct EmailSender {
    config: SmtpConfig,
}

impl EmailSender {
    /// Build a sender from resolved `SMTP` config.
    #[must_use]
    pub fn new(config: SmtpConfig) -> Self {
        Self { config }
    }

    /// Send an email, matching `sendEmail(to, subject, html)`.
    ///
    /// Validation order, matching the TypeScript exactly:
    /// 1. `SMTP_HOST` unset → `{ ok: false }` with
    ///    [`DeliveryFailure::EmailNotConfigured`], without attempting
    ///    anything else.
    /// 2. `to` empty or missing `@` → [`DeliveryFailure::EmailAddressRefused`].
    /// 3. Otherwise, connect and send; a failure is classified into
    ///    [`DeliveryFailure::MailServerUnreachable`] or
    ///    [`DeliveryFailure::MailServerRefused`] (`SRC-17` F1: the
    ///    transport's own message is never returned).
    ///
    /// # Errors
    ///
    /// Never returns `Err` — every failure mode is reported via
    /// `DeliverResult { ok: false, .. }`, matching the TypeScript, which
    /// never throws from `sendEmail`.
    pub async fn send(&self, to: &str, subject: &str, html: &str) -> DeliverResult {
        let Some(host) = &self.config.host else {
            return DeliverResult::err(DeliveryFailure::EmailNotConfigured);
        };
        if to.is_empty() || !to.contains('@') {
            return DeliverResult::err(DeliveryFailure::EmailAddressRefused);
        }

        let Ok(message) = build_message(&self.config.from, to, subject, html) else {
            return DeliverResult::err(DeliveryFailure::EmailAddressRefused);
        };

        let builder = if self.config.secure {
            AsyncSmtpTransport::<Tokio1Executor>::relay(host)
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)
        };
        let Ok(builder) = builder else {
            return DeliverResult::err(DeliveryFailure::MailServerUnreachable);
        };
        let mut builder = builder.port(self.config.port);
        if let Some(user) = &self.config.user {
            builder = builder.credentials(Credentials::new(user.clone(), self.config.pass.clone()));
        }
        let transport = builder.build();

        match transport.send(message).await {
            Ok(_) => DeliverResult::ok(),
            // `SRC-17` F1: an `SMTP` reply (transient or permanent) means
            // the server answered and refused; anything else (connect, TLS,
            // timeout) means it was not reached. The reply text is dropped.
            Err(err) if err.is_permanent() || err.is_transient() => {
                DeliverResult::err(DeliveryFailure::MailServerRefused)
            }
            Err(_) => DeliverResult::err(DeliveryFailure::MailServerUnreachable),
        }
    }
}

/// Dispatch to the right channel, matching `deliver(channel, target,
/// title, text)`.
///
/// For `"email"`, `text` is wrapped in a small HTML envelope — reproduced
/// here exactly, including the TypeScript's `text ? html : html` (both
/// branches of that ternary are identical, so the email body is always the
/// HTML envelope regardless of whether `text` is truthy; this looks like a
/// leftover from an earlier plain-text/HTML branch that was never finished
/// and is kept as-is for fidelity).
///
/// # Errors
///
/// Never returns `Err` — see [`WebhookSender::send`]/[`EmailSender::send`].
pub async fn deliver(
    http: &WebhookSender,
    email: &EmailSender,
    channel: &str,
    target: &str,
    title: &str,
    text: &str,
) -> DeliverResult {
    match channel {
        "webhook" => http.send(target, title, text).await,
        "email" => {
            let html = format!(
                "<div style=\"font-family:system-ui,sans-serif\"><h3 style=\"margin:0 0 8px\">{title}</h3><pre style=\"white-space:pre-wrap;font:inherit;margin:0\">{text}</pre><p style=\"color:#888;font-size:12px;margin-top:16px\">Rantai Lake — Enterprise Lakehouse Console</p></div>"
            );
            // `text ? html : html` in the TypeScript — both branches are
            // the literal same expression (apparently a leftover from an
            // earlier plain-text/HTML split that was never finished), so
            // the email body is always `html` regardless of `text`;
            // collapsed to that here since there is no behavioral
            // difference left to reproduce.
            email.send(target, title, &html).await
        }
        _ => DeliverResult::err(DeliveryFailure::ChannelUnsupported),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::json;
    use wiremock::matchers::{body_json, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    #[tokio::test]
    async fn webhook_rejects_non_http_url() {
        let client = MappedResolver::new().into_sender();
        let result = client.send("ftp://example.com", "t", "x").await;
        assert_eq!(
            result,
            DeliverResult::err(DeliveryFailure::WebhookAddressRefused)
        );
    }

    #[tokio::test]
    async fn webhook_posts_expected_body_shape() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_json(json!({
                "text": "*Alert*\nsomething happened",
                "content": "**Alert**\nsomething happened",
                "title": "Alert",
                "message": "something happened",
            })))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let client = MappedResolver::new().into_sender();
        let result = client
            .send(&server.uri(), "Alert", "something happened")
            .await;
        assert_eq!(result, DeliverResult::ok());
    }

    #[tokio::test]
    async fn webhook_reports_non_2xx_status() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let client = MappedResolver::new().into_sender();
        let result = client.send(&server.uri(), "t", "x").await;
        assert_eq!(
            result,
            DeliverResult::err(DeliveryFailure::WebhookServerError)
        );
        // SRC-17 F1: the status code is not in the text.
        assert!(!result.error.unwrap().contains("500"));
    }

    /// `SEC-10`: the connection goes to the address the resolver approved,
    /// not to wherever the name resolves. `pinned.test` is a reserved name
    /// that resolves nowhere, so a delivery can only succeed if the request
    /// was pinned to the mapped listener.
    #[tokio::test]
    async fn a_named_webhook_connects_to_the_approved_address_not_to_dns() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let addr = *server.address();
        let sender = MappedResolver::new()
            .with("pinned.test", addr)
            .into_sender();
        let url = format!("http://pinned.test:{}/hook", addr.port());
        assert_eq!(
            sender.send(&url, "t", "x").await,
            DeliverResult::ok(),
            "the request must reach the pinned address"
        );
    }

    /// `SEC-10`: a redirect is a failed delivery and the second hop is never
    /// requested.
    #[tokio::test]
    async fn a_redirect_is_a_failed_delivery_and_is_not_followed() {
        let second = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&second)
            .await;
        let first = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(302).insert_header("Location", second.uri().as_str()),
            )
            .mount(&first)
            .await;
        let sender = MappedResolver::new().into_sender();
        let result = sender.send(&first.uri(), "t", "x").await;
        assert_eq!(
            result,
            DeliverResult::err(DeliveryFailure::WebhookRedirected)
        );
        assert!(
            second.received_requests().await.unwrap().is_empty(),
            "the redirect target must never be requested"
        );
    }

    /// `SEC-10`: a refusal from the resolver is a fixed message, the
    /// listener is never contacted, and a URL with user info or a missing
    /// host is refused before any lookup.
    #[tokio::test]
    async fn an_unresolvable_or_malformed_target_gets_a_fixed_message() {
        let sender = MappedResolver::new().into_sender();
        assert_eq!(
            sender.send("http://nowhere.test/x", "t", "x").await,
            DeliverResult::err(DeliveryFailure::WebhookUnreachable)
        );
        for bad in [
            "http://user:pw@example.test/x",
            "http://user@example.test/x",
            "http://:80/x",
            "not a url",
            "file:///etc/passwd",
        ] {
            assert_eq!(
                sender.send(bad, "t", "x").await,
                DeliverResult::err(DeliveryFailure::WebhookAddressRefused),
                "{bad}"
            );
        }
    }

    /// `SEC-10`: no result string carries the HTTP library's text. A closed
    /// port makes `reqwest` fail with an error whose own text names the URL;
    /// the delivery record must hold only the fixed message.
    #[tokio::test]
    async fn a_connection_failure_reports_fixed_text_not_the_library_error() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let sender = MappedResolver::new().into_sender();
        let url = format!("http://127.0.0.1:{port}/secret-marker-token");
        let result = sender.send(&url, "t", "x").await;
        assert_eq!(
            result,
            DeliverResult::err(DeliveryFailure::WebhookUnreachable)
        );
        let text = result.error.unwrap();
        assert!(!text.contains("secret-marker-token"), "{text}");
        assert!(!text.contains("error sending request"), "{text}");
    }

    /// `SEC-10`: the pre-check used when a rule is saved agrees with the
    /// send path and sends nothing.
    #[tokio::test]
    async fn check_validates_a_url_without_sending() {
        let server = MockServer::start().await;
        let sender = MappedResolver::new().into_sender();
        assert_eq!(sender.check(&server.uri()).await, Ok(()));
        assert_eq!(
            sender.check("ftp://example.test").await,
            Err(WebhookError::InvalidUrl)
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    fn cfg(host: Option<&str>) -> SmtpConfig {
        SmtpConfig {
            host: host.map(str::to_owned),
            port: 587,
            secure: false,
            user: Some("bot@example.com".to_owned()),
            pass: "hunter2".to_owned(),
            from: "rantai-lake@localhost".to_owned(),
        }
    }

    #[tokio::test]
    async fn email_reports_unconfigured_smtp() {
        let sender = EmailSender::new(cfg(None));
        let result = sender.send("someone@example.com", "s", "<p>hi</p>").await;
        assert_eq!(
            result,
            DeliverResult::err(DeliveryFailure::EmailNotConfigured)
        );
    }

    #[tokio::test]
    async fn email_rejects_invalid_address() {
        let sender = EmailSender::new(cfg(Some("smtp.example.com")));
        let result = sender.send("not-an-email", "s", "<p>hi</p>").await;
        assert_eq!(
            result,
            DeliverResult::err(DeliveryFailure::EmailAddressRefused)
        );

        let result_empty = sender.send("", "s", "<p>hi</p>").await;
        assert_eq!(
            result_empty,
            DeliverResult::err(DeliveryFailure::EmailAddressRefused)
        );
    }

    #[test]
    fn build_message_sets_headers_from_to_subject_and_html_body() {
        let message = build_message(
            "rantai-lake@localhost",
            "someone@example.com",
            "Nightly digest",
            "<p>hello</p>",
        )
        .unwrap();
        let headers = message.headers();
        assert_eq!(headers.get_raw("Subject").unwrap(), "Nightly digest");
        assert!(
            headers
                .get_raw("From")
                .unwrap()
                .contains("rantai-lake@localhost")
        );
        assert!(
            headers
                .get_raw("To")
                .unwrap()
                .contains("someone@example.com")
        );
        let body = String::from_utf8(message.formatted()).unwrap();
        assert!(body.contains("<p>hello</p>"));
        assert!(body.contains("Content-Type: text/html"));
    }

    #[tokio::test]
    async fn deliver_dispatches_webhook_channel() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let http = MappedResolver::new().into_sender();
        let email = EmailSender::new(cfg(None));
        let result = deliver(&http, &email, "webhook", &server.uri(), "t", "x").await;
        assert_eq!(result, DeliverResult::ok());
    }

    #[tokio::test]
    async fn deliver_dispatches_email_channel_and_still_validates_smtp() {
        let http = MappedResolver::new().into_sender();
        let email = EmailSender::new(cfg(None));
        let result = deliver(&http, &email, "email", "someone@example.com", "t", "x").await;
        assert_eq!(
            result,
            DeliverResult::err(DeliveryFailure::EmailNotConfigured)
        );
    }

    #[tokio::test]
    async fn deliver_rejects_unknown_channel() {
        let http = MappedResolver::new().into_sender();
        let email = EmailSender::new(cfg(None));
        let result = deliver(&http, &email, "carrier-pigeon", "target", "t", "x").await;
        assert_eq!(
            result,
            DeliverResult::err(DeliveryFailure::ChannelUnsupported)
        );
        // SRC-17 F1: the rule's channel text is not echoed.
        assert!(!result.error.unwrap().contains("carrier-pigeon"));
    }

    /// SRC-17 F1 (D-a): a webhook host nothing listens on gives the fixed
    /// "could not be reached" reason, and nothing from the driver's error
    /// (which names the address) is in it.
    #[tokio::test]
    async fn webhook_to_an_unreachable_host_gives_the_fixed_reason_and_no_driver_text() {
        // Bind then drop, so the port is free and the connect is refused.
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let client = MappedResolver::new().into_sender();
        let url = format!("http://127.0.0.1:{port}/hook");
        let result = client.send(&url, "t", "x").await;
        assert_eq!(
            result,
            DeliverResult::err(DeliveryFailure::WebhookUnreachable)
        );
        let text = result.error.unwrap();
        assert!(!text.contains("127.0.0.1") && !text.contains(&port.to_string()));
    }

    /// SRC-17 F1 (D-a): a mail server nothing listens on gives the fixed
    /// "could not be reached" reason whether the transport fails to build or
    /// fails to connect, and none of the driver's text.
    #[tokio::test]
    async fn email_to_an_unreachable_server_gives_the_fixed_reason_and_no_driver_text() {
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let mut config = cfg(Some("127.0.0.1"));
        config.port = port;
        let sender = EmailSender::new(config);
        let result = sender.send("someone@example.com", "s", "<p>hi</p>").await;
        assert_eq!(
            result,
            DeliverResult::err(DeliveryFailure::MailServerUnreachable)
        );
        assert!(!result.error.unwrap().contains("127.0.0.1"));
    }

    /// SRC-17 F1: the set of reasons is closed and every sentence is pinned,
    /// so a new one cannot be added without a test noticing.
    #[test]
    fn every_delivery_failure_has_its_fixed_sentence() {
        let all = [
            (
                DeliveryFailure::WebhookAddressRefused,
                "the webhook address was refused",
            ),
            (
                DeliveryFailure::WebhookUnreachable,
                "the webhook server could not be reached",
            ),
            (
                DeliveryFailure::WebhookServerError,
                "the webhook server answered with an error",
            ),
            (
                DeliveryFailure::WebhookRedirected,
                "the webhook server redirected the request",
            ),
            (
                DeliveryFailure::EmailNotConfigured,
                "e-mail is not set up on this installation",
            ),
            (
                DeliveryFailure::EmailAddressRefused,
                "the e-mail address was refused",
            ),
            (
                DeliveryFailure::MailServerUnreachable,
                "the mail server could not be reached",
            ),
            (
                DeliveryFailure::MailServerRefused,
                "the mail server refused the message",
            ),
            (
                DeliveryFailure::ChannelUnsupported,
                "the rule's channel is not supported",
            ),
        ];
        for (failure, sentence) in all {
            assert_eq!(failure.message(), sentence);
        }
    }
}
