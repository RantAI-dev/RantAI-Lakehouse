//! `SEC-41` decision 7 / AC1: `sqlx` 0.9 can dial with
//! [`PgSslMode::VerifyCa`] to an **IP address** when the server's certificate
//! names a **host**, and still refuses a certificate from another authority.
//!
//! # Why this test exists
//!
//! `SEC-14` part B dials a connector's database at the approved IP address
//! (`SEC-15`), not the name, so with a CA configured it needs "verify the
//! chain, skip the name". `sqlx-core` 0.8.6 meant to do that but only
//! recognised rustls' `NotValidForName`; rustls 0.23.46 reports
//! `NotValidForNameContext`, so every such dial failed (SEC-41's spec, "Why").
//! `sqlx-core` 0.9.0 recognises both. This test pins that half of the fix
//! down; the production `VerifyCa` call itself lands with `SEC-14` part B
//! (#192), which rebases on `SEC-41`.
//!
//! # The TLS terminator
//!
//! The shared test Postgres (`lakehouse-test-support`) answers `SSLRequest`
//! with `N` (`SHOW ssl` is off), so nothing can verify a certificate against
//! it. This test therefore puts a TLS terminator in front of it: answer
//! Postgres' 8-byte `SSLRequest` with `S`, run a rustls handshake with a
//! certificate minted per test, then forward the decrypted bytes to the real
//! server. Everything behind it is a REAL Postgres. The shape is copied from
//! `SEC-14` part B's `rust/crates/lakehouse-api/src/tls_proxy.rs`
//! (`origin/fix/sec-14-part-b-tls`), trimmed to one certificate chain and no
//! fixed name constant.
//!
//! The two authorities are minted at test time, never committed: a committed
//! private key is a leak (`.gitleaksignore` history, `SEC-36`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use base64::Engine as _;
use sqlx::Connection;
use sqlx::PgConnection;
use sqlx::postgres::{PgConnectOptions, PgSslMode};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

/// The 8 bytes of a `PostgreSQL` `SSLRequest` frame: its own length (8) and
/// the request code (80877103), big-endian. A client sends this before any
/// handshake and reads the next byte as `S` (upgrade) or `N` (plaintext).
const SSL_REQUEST: [u8; 8] = [0, 0, 0, 8, 4, 210, 22, 47];

/// The name the server's certificate carries, while the dial goes to
/// `127.0.0.1` — the mismatch `VerifyCa` must tolerate (and the reason
/// `NotValidForNameContext` matters).
const CERTIFICATE_NAME: &str = "db.internal";

/// A certificate authority minted for one test.
struct TestCa {
    certificate: rcgen::Certificate,
    key: rcgen::KeyPair,
}

impl TestCa {
    /// A fresh CA, valid to sign server certificates.
    fn new(common_name: &str) -> Self {
        let mut params = rcgen::CertificateParams::new(Vec::new()).expect("CA parameters");
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, common_name);
        let key = rcgen::KeyPair::generate().expect("a CA key pair");
        let certificate = params.self_signed(&key).expect("a self-signed CA");
        Self { certificate, key }
    }

    /// A server certificate for [`CERTIFICATE_NAME`], signed by this CA.
    fn server_certificate(&self) -> (rcgen::Certificate, rcgen::KeyPair) {
        let mut params = rcgen::CertificateParams::new(vec![CERTIFICATE_NAME.to_owned()])
            .expect("server params");
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, CERTIFICATE_NAME);
        let key = rcgen::KeyPair::generate().expect("a server key pair");
        let certificate = params
            .signed_by(&key, &self.certificate, &self.key)
            .expect("a certificate signed by the test CA");
        (certificate, key)
    }

    /// The CA as a PEM `CERTIFICATE` block, the shape
    /// `PgConnectOptions::ssl_root_cert_from_pem` takes.
    fn pem(&self) -> String {
        pem_certificate(&self.certificate)
    }
}

/// Wrap a DER certificate as a PEM `CERTIFICATE` block. `rcgen` is built
/// without its `pem` feature (it would add the `pem` crate for this one
/// function); `base64` is already a dependency of this crate.
fn pem_certificate(certificate: &rcgen::Certificate) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(certificate.der().as_ref());
    let mut out = String::from("-----BEGIN CERTIFICATE-----\n");
    for line in encoded.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(line).expect("base64 of a certificate is ASCII"));
        out.push('\n');
    }
    out.push_str("-----END CERTIFICATE-----\n");
    out
}

/// A listener that terminates TLS and forwards each connection's plaintext
/// to one plain server. Stopped when the test ends (the task aborts with the
/// process; the port is the operating system's choice on loopback).
async fn start_tls_terminator(
    plain_host: &str,
    plain_port: u16,
    certificate: rcgen::Certificate,
    key: rcgen::KeyPair,
) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback listener for the TLS terminator");
    let port = listener
        .local_addr()
        .expect("the bound terminator address")
        .port();

    let key_der = rustls::pki_types::PrivateKeyDer::Pkcs8(
        rustls::pki_types::PrivatePkcs8KeyDer::from(key.serialize_der()),
    );
    // `ring` named explicitly, as `sqlx-core`'s rustls backend does: the
    // process-level default provider is set by whoever gets there first, and
    // this workspace links more than one rustls consumer.
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("the TLS versions rustls supports by default")
    .with_no_client_auth()
    .with_single_cert(vec![certificate.der().clone()], key_der)
    .expect("the minted certificate and key form a usable server identity");
    let acceptor = TlsAcceptor::from(Arc::new(config));

    let plain = format!("{plain_host}:{plain_port}");
    tokio::spawn(async move {
        while let Ok((inbound, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            let plain = plain.clone();
            tokio::spawn(async move {
                let Some(inbound) = accept_ssl_upgrade(inbound).await else {
                    return;
                };
                let Ok(mut inbound) = acceptor.accept(inbound).await else {
                    return;
                };
                let Ok(mut outbound) = TcpStream::connect(plain.as_str()).await else {
                    return;
                };
                let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
            });
        }
    });

    port
}

/// Answer one `SSLRequest` with `S` and hand the socket back for the TLS
/// handshake. A server that skipped this would read a `PostgreSQL` frame
/// where a `ClientHello` belongs.
async fn accept_ssl_upgrade(mut inbound: TcpStream) -> Option<TcpStream> {
    let mut frame = [0u8; 8];
    inbound.read_exact(&mut frame).await.ok()?;
    if frame != SSL_REQUEST {
        return None;
    }
    inbound.write_all(b"S").await.ok()?;
    inbound.flush().await.ok()?;
    Some(inbound)
}

/// Install `ring` as rustls' process-wide default provider, once.
///
/// # Why this is needed at all (finding for `SEC-14` part B)
///
/// `sqlx-core` 0.9's `VerifyCa` path builds its verifier with
/// `WebPkiServerVerifier::builder(...)`
/// (`sqlx-core-0.9.0/src/net/tls/tls_rustls.rs:155`), which reads rustls'
/// **process-level** default provider. This workspace links two rustls
/// providers into the API binary — `ring` through `sqlx-core`'s
/// `tls-rustls`, and `aws-lc-rs` through `tiberius`/`tokio-rustls`'s default
/// features — so rustls cannot pick one and panics ("Could not
/// automatically determine the process-level `CryptoProvider`") on the first
/// `VerifyCa` dial. This test installs `ring` for itself so the sqlx half can
/// be exercised; the API binary needs the same install (or a chosen provider
/// for every rustls consumer) before `SEC-14` part B's first production
/// `VerifyCa` dial, which is that item's decision, not this test's.
fn install_rustls_provider() {
    static ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        // Another test in this binary may have installed one first; either
        // provider verifies the same chain here, so a loser is not an error.
        let _installed = rustls::crypto::ring::default_provider().install_default();
    });
}

/// `SEC-41`-AC1: `VerifyCa` at the IP with a certificate for the name
/// connects and runs `SELECT 1`; the same dial against another CA fails.
#[tokio::test]
async fn verify_ca_at_an_ip_accepts_the_named_certificate_and_refuses_another_ca() {
    install_rustls_provider();

    // The shared test Postgres, which does not speak TLS itself. The URL is
    // `postgres://postgres:postgres@127.0.0.1:<port>/postgres`
    // (`lakehouse-test-support`), so its parts are all here.
    let admin = url::Url::parse(&lakehouse_test_support::database_url())
        .expect("the test-support DATABASE_URL parses");
    let plain_host = admin.host_str().expect("a host in DATABASE_URL").to_owned();
    let plain_port = admin.port().expect("a mapped port in DATABASE_URL");
    let user = admin.username().to_owned();
    let password = admin
        .password()
        .expect("a password in DATABASE_URL")
        .to_owned();
    let database = admin.path().trim_start_matches('/').to_owned();

    let ca = TestCa::new("SEC-41 test CA");
    let (server_certificate, server_key) = ca.server_certificate();
    let proxy_port =
        start_tls_terminator(&plain_host, plain_port, server_certificate, server_key).await;

    // The dial a connector's `VerifyCa` path makes: the IP the SSRF check
    // approved, never the name the certificate carries.
    let dial = |root_pem: String| {
        PgConnectOptions::new()
            .host("127.0.0.1")
            .port(proxy_port)
            .username(&user)
            .password(&password)
            .database(&database)
            .ssl_mode(PgSslMode::VerifyCa)
            .ssl_root_cert_from_pem(root_pem.into_bytes())
    };

    let mut connection = PgConnection::connect_with(&dial(ca.pem()))
        .await
        .expect("sqlx 0.9 must accept a chain for a name when dialled at an IP");
    let one: i32 = sqlx::query_scalar("SELECT 1")
        .fetch_one(&mut connection)
        .await
        .expect("SELECT 1 over the verified connection");
    assert_eq!(one, 1, "the verified connection must really run a query");
    connection
        .close()
        .await
        .expect("closing the verified connection");

    // The same dial, trusting a different authority: the chain does not
    // verify, so no connection is made. This half must fail on 0.8 too — a
    // check that cannot refuse is not a check. `is_err` is enough: the
    // positive half above already proved this very terminator, address and
    // credentials complete a handshake, so the only difference left between
    // the two dials is which CA they trust.
    let other_ca = TestCa::new("some other CA");
    let refused = PgConnection::connect_with(&dial(other_ca.pem())).await;
    assert!(
        refused.is_err(),
        "a certificate from another CA must not be accepted"
    );
}
