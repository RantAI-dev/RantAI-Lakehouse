//! A real relay, behind a private CA, in a container (`SEC-56` T2).
//!
//! The unit tests in `lakehouse-notify` prove that a bundle is read and
//! that it is added to the transport's root store; they cannot prove that
//! a handshake against a relay whose certificate chains to a private CA
//! actually succeeds. This file does: it starts a REAL `Mailpit` in a
//! container, mints a CA and a server certificate for it at run time, and
//! sends through [`lakehouse_notify::EmailSender`].
//!
//! # The container
//!
//! `axllent/mailpit`, pinned to an exact tag (`v1.33.0`), configured
//! through the environment variables its own documentation names
//! (<https://mailpit.axllent.org/docs/configuration/smtp/>):
//!
//! - `MP_SMTP_TLS_CERT` — the server certificate, in the container;
//! - `MP_SMTP_TLS_KEY` — the server's private key, in the container;
//! - `MP_SMTP_REQUIRE_STARTTLS=true` — the relay refuses a plaintext
//!   session and requires `STARTTLS`;
//! - `MP_SMTP_REQUIRE_TLS=true` — the relay speaks implicit TLS (the
//!   "SSL/TLS required" mode; the two modes cannot run on the same port).
//!
//! The certificate and the key are minted for the test and copied into the
//! container; no certificate and no key is ever committed to this
//! repository (`SEC-56` D8: the `.gitleaksignore` history, `SEC-36`, reads
//! a committed key as a leak).
//!
//! # What each case proves
//!
//! 1. a relay that requires `STARTTLS`, with no bundle, is a TLS failure
//!    and nothing is delivered;
//! 2. the same relay, with the CA as a bundle, delivers the message;
//! 3. a bundle with a DIFFERENT CA is a TLS failure and nothing is
//!    delivered;
//! 4. the right CA but a host that is not on the certificate is a TLS
//!    failure — the hostname check stays on (`SEC-56` D3);
//! 5. implicit TLS with the CA delivers the message;
//! 6. a bundle of two certificates in which the relay's CA is the second
//!    delivers the message.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use lakehouse_notify::{DeliveryFailure, EmailSender, SmtpCaBundle, SmtpConfig};
use testcontainers::core::WaitFor;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};

/// The relay, pinned to an exact tag.
const MAILPIT_IMAGE: &str = "axllent/mailpit";
const MAILPIT_TAG: &str = "v1.33.0";
/// The SMTP port inside the container.
const SMTP_PORT: u16 = 1025;
/// The HTTP API port inside the container.
const HTTP_PORT: u16 = 8025;
/// Where the certificate and the key are put inside the container.
const CERTIFICATE_PATH: &str = "/tls/server.pem";
const KEY_PATH: &str = "/tls/server.key";
/// The subject every case sends, so "the relay holds exactly that subject"
/// is one comparison.
const SUBJECT: &str = "relay reached over a private CA";
/// The recipient every case sends to.
const RECIPIENT: &str = "someone@example.com";

/// Which TLS the relay demands.
#[derive(Clone, Copy)]
enum TlsMode {
    /// `STARTTLS` is required: a plaintext session is refused.
    StarttlsRequired,
    /// Implicit TLS: the session is TLS from the first byte.
    TlsRequired,
}

/// The CA and the server certificate it signs, minted for one test.
struct TestPki {
    /// The CA certificate, PEM.
    ca_pem: String,
    /// The server certificate, PEM.
    certificate_pem: String,
    /// The server's private key, PEM.
    key_pem: String,
    /// The names the server certificate carries.
    names: Vec<String>,
}

impl TestPki {
    /// Whether the certificate carries `name`.
    fn carries(&self, name: &str) -> bool {
        self.names.iter().any(|carried| carried == name)
    }
}

/// The PEM armour around a DER certificate or key, written here so
/// `rcgen`'s `pem` feature stays off (turning it on would unify the
/// feature into `lakehouse-api`'s tests, which pin `rcgen` without it).
fn pem(label: &str, der: &[u8]) -> String {
    use base64::Engine as _;
    use std::fmt::Write as _;
    let encoded = base64::engine::general_purpose::STANDARD.encode(der);
    let mut out = format!("-----BEGIN {label}-----\n");
    for line in encoded.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(line).expect("base64 is ASCII"));
        out.push('\n');
    }
    let _ = writeln!(out, "-----END {label}-----");
    out
}

/// Mint a CA and a server certificate signed by it, for the given names.
///
/// The server certificate carries the names as SANs, so the relay's
/// certificate is valid for the host the sender dials — and for nothing
/// else, which is what case 4 needs.
fn mint_pki(names: &[&str]) -> TestPki {
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let mut ca_params = rcgen::CertificateParams::default();
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![
        rcgen::KeyUsagePurpose::KeyCertSign,
        rcgen::KeyUsagePurpose::CrlSign,
    ];
    let ca_cert = ca_params.self_signed(&ca_key).unwrap();

    let server_key = rcgen::KeyPair::generate().unwrap();
    let mut server_params = rcgen::CertificateParams::new(
        names
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    server_params.use_authority_key_identifier_extension = true;
    server_params.key_usages = vec![
        rcgen::KeyUsagePurpose::DigitalSignature,
        rcgen::KeyUsagePurpose::KeyEncipherment,
    ];
    server_params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
    let server_cert = server_params
        .signed_by(
            &server_key,
            &rcgen::Issuer::from_params(&ca_params, &ca_key),
        )
        .unwrap();

    TestPki {
        ca_pem: pem("CERTIFICATE", ca_cert.der().as_ref()),
        certificate_pem: pem("CERTIFICATE", server_cert.der().as_ref()),
        key_pem: pem("PRIVATE KEY", &server_key.serialize_der()),
        names: names.iter().map(|name| (*name).to_owned()).collect(),
    }
}

/// A running relay, and the addresses it is reached on.
struct Relay {
    /// Kept alive for the test: dropping it stops the container.
    _container: ContainerAsync<GenericImage>,
    /// The host `testcontainers` reports — the name the relay's certificate
    /// carries, and the name the sender dials.
    host: String,
    /// The host port the SMTP server is published on.
    smtp_port: u16,
    /// The host port the HTTP API is published on.
    http_port: u16,
    /// The CA and the certificate the relay serves.
    pki: TestPki,
}

impl Relay {
    /// The bundle that trusts the relay's CA.
    fn bundle(&self) -> SmtpCaBundle {
        SmtpCaBundle::from_pem(self.pki.ca_pem.as_bytes()).unwrap()
    }

    /// The subjects of the messages the relay holds.
    async fn subjects(&self) -> Vec<String> {
        let url = format!("http://{}:{}/api/v1/messages", self.host, self.http_port);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap();
        let body: serde_json::Value = client.get(url).send().await.unwrap().json().await.unwrap();
        body["messages"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|message| message["Subject"].as_str().unwrap_or_default().to_owned())
            .collect()
    }

    /// A name the relay is reachable on that its certificate does NOT
    /// carry: the loopback addresses the published port listens on.
    fn a_name_the_certificate_does_not_carry(&self) -> String {
        for candidate in ["127.0.0.1", "::1"] {
            if !self.pki.carries(candidate) {
                return candidate.to_owned();
            }
        }
        panic!(
            "the certificate carries every loopback address: {:?}",
            self.pki.names
        );
    }
}

/// Start the relay container, serving the given certificate.
async fn start_container(mode: TlsMode, pki: &TestPki) -> ContainerAsync<GenericImage> {
    let mut image = GenericImage::new(MAILPIT_IMAGE, MAILPIT_TAG)
        .with_wait_for(WaitFor::message_on_either_std("[smtpd] starting on"))
        .with_env_var("MP_SMTP_TLS_CERT", CERTIFICATE_PATH)
        .with_env_var("MP_SMTP_TLS_KEY", KEY_PATH)
        .with_copy_to(CERTIFICATE_PATH, pki.certificate_pem.clone().into_bytes())
        .with_copy_to(KEY_PATH, pki.key_pem.clone().into_bytes());
    image = match mode {
        TlsMode::StarttlsRequired => image.with_env_var("MP_SMTP_REQUIRE_STARTTLS", "true"),
        TlsMode::TlsRequired => image.with_env_var("MP_SMTP_REQUIRE_TLS", "true"),
    };
    image.start().await.unwrap()
}

/// Start the relay, with a certificate that carries the name the sender
/// dials.
///
/// That name is the host `testcontainers` reports, and it is a property of
/// the docker environment rather than of the container — so it is read
/// from a container that is already up. With a local docker daemon it is
/// "localhost", which the certificate already carries and the relay is
/// that container; in the rare setup where it is another name, the relay
/// is started again with a certificate that carries it.
async fn start_relay(mode: TlsMode) -> Relay {
    let mut pki = mint_pki(&["localhost"]);
    let mut container = start_container(mode, &pki).await;
    let host = container.get_host().await.unwrap().to_string();
    if !pki.carries(&host) {
        container.stop_with_timeout(Some(0)).await.unwrap();
        pki = mint_pki(&["localhost", &host]);
        container = start_container(mode, &pki).await;
    }
    let smtp_port = container.get_host_port_ipv4(SMTP_PORT).await.unwrap();
    let http_port = container.get_host_port_ipv4(HTTP_PORT).await.unwrap();
    let relay = Relay {
        _container: container,
        host,
        smtp_port,
        http_port,
        pki,
    };
    wait_for_the_smtp_port(&relay.host, relay.smtp_port).await;
    relay
}

/// Wait until the relay listens: a container that is up is not yet a relay
/// that answers, and a dial that arrives too early is a connection error,
/// not a TLS one.
async fn wait_for_the_smtp_port(host: &str, port: u16) {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        if tokio::net::TcpStream::connect((host, port)).await.is_ok() {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the relay never started listening on {host}:{port}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The sender a case uses: the relay's port, the host the case dials, the
/// TLS mode the case wants and the bundle the case trusts.
fn sender(relay: &Relay, host: &str, secure: bool, ca_bundle: Option<SmtpCaBundle>) -> EmailSender {
    EmailSender::new(SmtpConfig {
        host: Some(host.to_owned()),
        port: relay.smtp_port,
        secure,
        user: None,
        pass: String::new(),
        from: "sender@example.com".to_owned(),
        ca_bundle,
    })
}

/// `SEC-56` T2 case 1: a relay that requires `STARTTLS`, with no bundle,
/// is a TLS failure — and nothing is delivered.
#[tokio::test]
async fn a_relay_that_requires_starttls_fails_without_a_ca_bundle() {
    let relay = start_relay(TlsMode::StarttlsRequired).await;

    let result = sender(&relay, &relay.host, false, None)
        .send(RECIPIENT, SUBJECT, "<p>hi</p>")
        .await;

    assert_eq!(
        result.error.as_deref(),
        Some(DeliveryFailure::MailServerTlsFailed.message())
    );
    assert_eq!(relay.subjects().await, Vec::<String>::new());
}

/// `SEC-56` T2 case 2: the same relay, with the CA as a bundle, delivers
/// the message.
#[tokio::test]
async fn a_relay_that_requires_starttls_is_reached_with_its_ca_bundle() {
    let relay = start_relay(TlsMode::StarttlsRequired).await;

    let result = sender(&relay, &relay.host, false, Some(relay.bundle()))
        .send(RECIPIENT, SUBJECT, "<p>hi</p>")
        .await;

    assert!(result.ok, "{:?}", result.error);
    assert_eq!(relay.subjects().await, vec![SUBJECT.to_owned()]);
}

/// `SEC-56` T2 case 3: a bundle with a DIFFERENT CA is a TLS failure, and
/// nothing is delivered.
#[tokio::test]
async fn a_bundle_with_another_ca_fails_and_delivers_nothing() {
    let relay = start_relay(TlsMode::StarttlsRequired).await;
    let other = mint_pki(&["localhost"]);

    let result = sender(
        &relay,
        &relay.host,
        false,
        Some(SmtpCaBundle::from_pem(other.ca_pem.as_bytes()).unwrap()),
    )
    .send(RECIPIENT, SUBJECT, "<p>hi</p>")
    .await;

    assert_eq!(
        result.error.as_deref(),
        Some(DeliveryFailure::MailServerTlsFailed.message())
    );
    assert_eq!(relay.subjects().await, Vec::<String>::new());
}

/// `SEC-56` T2 case 4: the right CA but a host that is not on the
/// certificate is a TLS failure — the hostname check stays on (D3).
#[tokio::test]
async fn a_host_that_is_not_on_the_certificate_fails_the_handshake() {
    let relay = start_relay(TlsMode::StarttlsRequired).await;
    let wrong_name = relay.a_name_the_certificate_does_not_carry();

    let result = sender(&relay, &wrong_name, false, Some(relay.bundle()))
        .send(RECIPIENT, SUBJECT, "<p>hi</p>")
        .await;

    assert_eq!(
        result.error.as_deref(),
        Some(DeliveryFailure::MailServerTlsFailed.message())
    );
    assert_eq!(relay.subjects().await, Vec::<String>::new());
}

/// `SEC-56` T2 case 5: implicit TLS (`SMTP_SECURE` in effect) with the CA
/// delivers the message.
#[tokio::test]
async fn a_relay_that_demands_tls_is_reached_with_its_ca_bundle() {
    let relay = start_relay(TlsMode::TlsRequired).await;

    let result = sender(&relay, &relay.host, true, Some(relay.bundle()))
        .send(RECIPIENT, SUBJECT, "<p>hi</p>")
        .await;

    assert!(result.ok, "{:?}", result.error);
    assert_eq!(relay.subjects().await, vec![SUBJECT.to_owned()]);
}

/// `SEC-56` T2 case 6: a bundle of two certificates in which the relay's CA
/// is the second one delivers the message.
#[tokio::test]
async fn a_bundle_of_two_certificates_reaches_a_relay_signed_by_the_second() {
    let relay = start_relay(TlsMode::StarttlsRequired).await;
    let other = mint_pki(&["localhost"]);

    let mut pem = other.ca_pem.clone();
    pem.push_str(&relay.pki.ca_pem);
    let bundle = SmtpCaBundle::from_pem(pem.as_bytes()).unwrap();
    assert_eq!(bundle.len(), 2);

    let result = sender(&relay, &relay.host, false, Some(bundle))
        .send(RECIPIENT, SUBJECT, "<p>hi</p>")
        .await;

    assert!(result.ok, "{:?}", result.error);
    assert_eq!(relay.subjects().await, vec![SUBJECT.to_owned()]);
}
