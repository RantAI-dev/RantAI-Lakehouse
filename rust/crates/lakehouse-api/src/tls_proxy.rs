//! A TLS-terminating TCP proxy in front of the shared test `Postgres`, for
//! the tests that exercise a production `PostgreSQL` dial (`SEC-14` part B).
//!
//! # Why this exists
//!
//! Since `SEC-14` part B every connector `PostgreSQL` dial REQUIRES TLS
//! (`D5`: `sslmode=require` at least), so a test that dials through the
//! production helper needs a server that answers `SSLRequest` with `S`. The
//! shared test server (`lakehouse-test-support`, `postgres:16-alpine`)
//! answers `N`: `SHOW ssl` is `off` in it. That container is the one every
//! test in the workspace runs against, so it is left as it is rather than
//! reconfigured for one crate's sake.
//!
//! [`TlsProxy::start`] therefore terminates TLS in front of it, with a
//! self-signed certificate minted at test time for `localhost`, and
//! forwards the decrypted bytes to the plain socket. Everything behind it is
//! a REAL `Postgres` -- replication slots, publications, `SCRAM-SHA-256`
//! authentication -- so the code under test still runs end to end against a
//! real server; only the TLS handshake belongs to the test double. That
//! handshake is what `pg_probe`'s own tests pin down, and the compose stack
//! proves it against a `Postgres` that really serves TLS.
//!
//! No network beyond loopback and no fixed port: the listener binds
//! `127.0.0.1:0` and the certificate is generated per proxy, so nothing is
//! shared between tests and nothing is written outside the test process.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_rustls::TlsAcceptor;

/// The 8 bytes of a `PostgreSQL` `SSLRequest` frame: its own length (8) and
/// the request code (80877103) as big-endian `i32`s. The proxy answers it
/// with `S` the way a TLS-terminating server must -- a client sends this
/// before it starts a handshake, and reads any first byte that is not `S` or
/// `N` as a protocol error, a TLS alert record included.
const SSL_REQUEST: [u8; 8] = [0, 0, 0, 8, 4, 210, 22, 47];

/// Answer one `SSLRequest` with `S` and hand the socket back for the TLS
/// handshake. A server that skipped this would be reading a `PostgreSQL`
/// frame where a `ClientHello` belongs.
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

/// The name the proxy's own certificate carries, so a test can ask for
/// `verify-full` against it (a name the certificate does NOT carry is the
/// refusal case `pg_probe` tests).
pub(crate) const PROXY_SERVER_NAME: &str = "localhost";

/// A listener that terminates TLS and forwards each connection's plaintext
/// to one plain server. Stopped when it is dropped.
pub(crate) struct TlsProxy {
    port: u16,
    certificate_pem: String,
    accept_task: JoinHandle<()>,
}

impl TlsProxy {
    /// Start a proxy in front of `plain_host:plain_port` on a loopback port
    /// the operating system picks.
    pub(crate) async fn start(plain_host: &str, plain_port: u16) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a loopback listener for the TLS proxy");
        let port = listener
            .local_addr()
            .expect("the bound proxy address")
            .port();

        let (acceptor, certificate_pem) = self_signed_acceptor();
        // Owned before the task, so the spawned future borrows nothing from
        // the caller.
        let plain = format!("{plain_host}:{plain_port}");
        let accept_task = tokio::spawn(async move {
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

        Self {
            port,
            certificate_pem,
            accept_task,
        }
    }

    /// The loopback port to dial instead of the plain server's.
    pub(crate) fn port(&self) -> u16 {
        self.port
    }

    /// The proxy's own certificate as a PEM `CERTIFICATE` block, for a test
    /// that needs a CA file to verify against.
    pub(crate) fn certificate_pem(&self) -> &str {
        &self.certificate_pem
    }
}

impl Drop for TlsProxy {
    fn drop(&mut self) {
        self.accept_task.abort();
    }
}

/// A [`TlsAcceptor`] for a freshly generated self-signed certificate, plus
/// that certificate as PEM. `ring` is named explicitly, as in
/// `pg_probe::client_config`: the default provider is process-global and
/// can be unset when several rustls consumers are linked in.
fn self_signed_acceptor() -> (TlsAcceptor, String) {
    let certified = rcgen::generate_simple_self_signed(vec![PROXY_SERVER_NAME.to_owned()])
        .expect("a self-signed certificate for the test proxy");
    let certificate = certified.cert.der().clone();
    let key = rustls::pki_types::PrivateKeyDer::Pkcs8(rustls::pki_types::PrivatePkcs8KeyDer::from(
        certified.key_pair.serialize_der(),
    ));
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("the TLS versions rustls supports by default")
    .with_no_client_auth()
    .with_single_cert(vec![certificate.clone()], key)
    .expect("the generated certificate and key form a usable server identity");
    (
        TlsAcceptor::from(Arc::new(config)),
        pem_certificate(&certificate),
    )
}

/// Wrap a DER certificate as a PEM `CERTIFICATE` block.
pub(crate) fn pem_certificate(der: &rustls::pki_types::CertificateDer<'_>) -> String {
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::STANDARD.encode(der.as_ref());
    let mut out = String::from("-----BEGIN CERTIFICATE-----\n");
    for line in encoded.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(line).expect("base64 of a certificate is ASCII"));
        out.push('\n');
    }
    out.push_str("-----END CERTIFICATE-----\n");
    out
}

#[cfg(test)]
mod tests {
    use rustls::pki_types::pem::PemObject;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    /// The proxy must really terminate TLS, or every test that puts it in
    /// front of a database proves nothing: this drives a plain echo server
    /// through it with a client that VERIFIES the proxy's own certificate
    /// (as a `CONNECTOR_TLS_CA_FILE` would), and reads back what it sent.
    #[tokio::test]
    async fn the_proxy_terminates_tls_with_the_certificate_it_advertises() {
        let echo = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let echo_port = echo.local_addr().unwrap().port();
        let echo_task = tokio::spawn(async move {
            let (mut socket, _) = echo.accept().await.unwrap();
            let mut buf = [0u8; 4];
            socket.read_exact(&mut buf).await.unwrap();
            socket.write_all(&buf).await.unwrap();
        });

        let proxy = TlsProxy::start("127.0.0.1", echo_port).await;

        let mut roots = rustls::RootCertStore::empty();
        for certificate in
            rustls::pki_types::CertificateDer::pem_slice_iter(proxy.certificate_pem().as_bytes())
        {
            roots.add(certificate.unwrap()).unwrap();
        }
        assert_eq!(roots.len(), 1, "the proxy advertises exactly one CA");
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
        let name = rustls::pki_types::ServerName::try_from(PROXY_SERVER_NAME.to_owned()).unwrap();
        let mut tcp = TcpStream::connect(("127.0.0.1", proxy.port()))
            .await
            .unwrap();
        // The `SSLRequest` a `PostgreSQL` client sends before any handshake,
        // so this test drives the proxy exactly the way a dial does.
        tcp.write_all(&SSL_REQUEST).await.unwrap();
        let mut answer = [0u8; 1];
        tcp.read_exact(&mut answer).await.unwrap();
        assert_eq!(&answer, b"S", "the proxy must accept the TLS upgrade");
        let mut tls = tokio_rustls::TlsConnector::from(Arc::new(config))
            .connect(name, tcp)
            .await
            .expect("the proxy must complete a verifiable handshake");

        tls.write_all(b"ping").await.unwrap();
        let mut echoed = [0u8; 4];
        tls.read_exact(&mut echoed).await.unwrap();
        assert_eq!(&echoed, b"ping");
        echo_task.await.unwrap();
    }
}
