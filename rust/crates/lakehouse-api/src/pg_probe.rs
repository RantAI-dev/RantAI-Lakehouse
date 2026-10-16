//! A minimal `PostgreSQL` client for the connection test (`SEC-14` part B,
//! decision D8, E2), used only by [`crate::connector_probe`]'s Postgres
//! dial.
//!
//! # Why this exists instead of `sqlx`
//!
//! `sqlx` cannot be told which authentication method it may use. Handed a
//! password and asked to connect, it answers a server's
//! `AuthenticationCleartextPassword` with the password in the clear and its
//! `AuthenticationMD5Password` with an MD5 hash — so a `/test` against a
//! server an operator points the console at could be made to hand a stored
//! connector credential to that server, or to a man in the middle, in a form
//! it can read back (`SEC-14-AC2`). This module speaks just enough of the
//! wire protocol to test a connection while refusing every method but
//! `SCRAM-SHA-256`, and it always requires TLS before the startup packet.
//!
//! # What it does, in order
//!
//! 1. Opens TCP to the [`Approved`] address (`SEC-15`: the address that was
//!    checked, never the name again) with [`connect_pinned`].
//! 2. Sends `SSLRequest`. A server that answers anything but `S` is refused
//!    with the fixed "TLS error" class **before any startup packet is
//!    sent** — the credential can never leave the process over a plaintext
//!    channel.
//! 3. Completes TLS with `tokio-rustls`. The name checked is the configured
//!    host name, never the pinned IP. With
//!    [`crate::config::Config::connector_tls_ca`]: the chain is verified
//!    against that CA **and** the name (verify-full). Without it: the
//!    connection is encrypted but the certificate is accepted as-is —
//!    [`AcceptAnyServerCert`] is named and documented as exactly that
//!    (D5's `require`).
//! 4. Sends the startup packet, then authenticates. Only
//!    `SCRAM-SHA-256` (through `postgres-protocol`) is performed;
//!    `AuthenticationCleartextPassword`, `AuthenticationMD5Password` and
//!    every other method are refused with one fixed sentence, and no
//!    `PasswordMessage` is ever written for them.
//! 5. Runs `SELECT 1` (a constant string, `postgres-protocol`'s own
//!    serialization) and terminates.
//!
//! # What it never does
//!
//! No upstream message text (a server `ErrorResponse`, a TLS error, a
//! `postgres-protocol` parse error) is ever formatted into a response — the
//! fixed classes in [`PgProbeError`]'s `Display` are all a caller sees
//! (`SEC-11`). The password stays a [`SecretValue`] until step 4, which
//! borrows it for the SCRAM computation and nothing else; it is never
//! logged, echoed, or copied into an error.

use std::fmt;
use std::path::Path;
use std::sync::Arc;

use bytes::BytesMut;
use fallible_iterator::FallibleIterator;
use lakehouse_core::secret::SecretValue;
use postgres_protocol::authentication::sasl::{ChannelBinding, SCRAM_SHA_256, ScramSha256};
use postgres_protocol::message::backend::{AuthenticationSaslBody, Message};
use postgres_protocol::message::frontend;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::CertificateDer;
use rustls::pki_types::ServerName;
use rustls::pki_types::UnixTime;
use rustls::pki_types::pem::PemObject;
use rustls::{DigitallySignedStruct, RootCertStore, SignatureScheme};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

use crate::connector_probe::{Approved, PgDialTarget, connect_pinned};

/// Why [`connect_and_select`] stopped. Every variant renders as one fixed,
/// generic sentence — never upstream text (`SEC-11`) — so a refusal is safe
/// to surface in the `/test` response.
#[derive(Debug)]
pub(crate) enum PgProbeError {
    /// The server did not complete the TLS upgrade: it answered `SSLRequest`
    /// with something other than `S`, or the handshake failed. Class:
    /// "TLS error". Raised before any startup packet, so no credential moved.
    Tls,
    /// The server asked for a password method this console refuses
    /// (`AuthenticationCleartextPassword`, `AuthenticationMD5Password`, or
    /// any other). No `PasswordMessage` is written for it.
    RefusedAuthMethod,
    /// The server offered SASL but not `SCRAM-SHA-256`.
    NoScram,
    /// The server rejected the credential, or the SCRAM proof did not verify.
    Authentication,
    /// The host refused the TCP connection.
    ConnectionRefused,
    /// The dial did not complete inside the timeout.
    TimedOut,
    /// Any other transport failure.
    Connection,
    /// The server reported an error running the query.
    Query,
    /// The server sent a message this probe does not understand.
    Protocol,
}

impl fmt::Display for PgProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Tls => "TLS error",
            Self::RefusedAuthMethod => {
                "server asked for an authentication method this console refuses"
            }
            Self::NoScram => "server did not offer SCRAM-SHA-256",
            Self::Authentication => "authentication failed",
            Self::ConnectionRefused => "connection refused",
            Self::TimedOut => "timed out",
            Self::Connection => "connection failed",
            Self::Query => "database rejected the connection",
            Self::Protocol => "the server sent a message this probe did not understand",
        };
        f.write_str(text)
    }
}

impl From<std::io::Error> for PgProbeError {
    fn from(err: std::io::Error) -> Self {
        match err.kind() {
            std::io::ErrorKind::ConnectionRefused => Self::ConnectionRefused,
            std::io::ErrorKind::TimedOut => Self::TimedOut,
            _ => Self::Connection,
        }
    }
}

/// Test a `PostgreSQL` connection: TLS, then a `SCRAM-SHA-256` sign-in, then
/// `SELECT 1`. `approved` is the address [`crate::connector_probe`] checked;
/// `target.host` is the configured name, kept for the certificate check.
///
/// The caller bounds the whole thing with `DIAL_TIMEOUT`; nothing here waits
/// unbounded on the network.
///
/// # Errors
///
/// Returns a [`PgProbeError`] for every refusal and every transport failure;
/// its `Display` is a fixed sentence safe to surface (`SEC-11`).
pub(crate) async fn connect_and_select(
    target: &PgDialTarget<'_>,
    approved: &Approved,
    password: &SecretValue,
    ca: Option<&Path>,
) -> Result<(), PgProbeError> {
    let tcp = connect_pinned(approved).await?;
    let mut stream = negotiate_tls(tcp, target.host, ca).await?;
    authenticate_and_select(&mut stream, target, password.expose_secret()).await
}

/// [`connect_and_select`]'s TLS half: `SSLRequest`, the single-byte answer,
/// then the `tokio-rustls` handshake over the same socket. Split out from
/// the authentication half so each can be refused for its own reason.
async fn negotiate_tls(
    tcp: TcpStream,
    host: &str,
    ca: Option<&Path>,
) -> Result<TlsStream<TcpStream>, PgProbeError> {
    let mut tcp = tcp;
    let mut request = BytesMut::new();
    frontend::ssl_request(&mut request);
    tcp.write_all(&request).await?;
    tcp.flush().await?;

    // Exactly one byte: a `read_exact` leaves anything the server already
    // sent after it in the socket, so the TLS handshake still sees the full
    // server hello.
    let mut answer = [0u8; 1];
    tcp.read_exact(&mut answer).await?;
    if answer[0] != b'S' {
        // The server would rather speak plaintext (or answered with an
        // error). Refuse before the startup packet — D5, `SEC-14-AC2`.
        return Err(PgProbeError::Tls);
    }

    let config = client_config(ca)?;
    let server_name = ServerName::try_from(host.to_owned()).map_err(|_| PgProbeError::Tls)?;
    let connector = TlsConnector::from(Arc::new(config));
    connector
        .connect(server_name, tcp)
        .await
        .map_err(|_| PgProbeError::Tls)
}

/// Build the client TLS configuration. `Some(ca)` verifies the chain against
/// that CA and the name (verify-full, D5's amended posture); `None` encrypts
/// with [`AcceptAnyServerCert`].
///
/// # Errors
///
/// Returns [`PgProbeError::Tls`] when the CA file cannot be read, holds no
/// certificate, or rustls rejects it: a configuration an operator cannot get
/// right must not fail once per connector with a TLS error.
fn client_config(ca: Option<&Path>) -> Result<rustls::ClientConfig, PgProbeError> {
    // `SEC-14` part B: pick the provider explicitly rather than through
    // `CryptoProvider::get_default()`, which is process-global and can be
    // unset or ambiguous when several rustls consumers are linked in.
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|_| PgProbeError::Tls)?;
    match ca {
        Some(path) => {
            let mut roots = RootCertStore::empty();
            let certificates =
                CertificateDer::pem_file_iter(path).map_err(|_| PgProbeError::Tls)?;
            for certificate in certificates {
                roots
                    .add(certificate.map_err(|_| PgProbeError::Tls)?)
                    .map_err(|_| PgProbeError::Tls)?;
            }
            if roots.is_empty() {
                // A CA file with no certificate in it would fail every dial
                // at the handshake; refuse the configuration here instead, so
                // the operator sees an unusable CA rather than a TLS error per
                // connector.
                return Err(PgProbeError::Tls);
            }
            Ok(builder.with_root_certificates(roots).with_no_client_auth())
        }
        None => Ok(builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAnyServerCert { provider }))
            .with_no_client_auth()),
    }
}

/// D5's `require` posture: encrypt, but do not verify. Named after exactly
/// what it does — it accepts ANY presented certificate and only checks that
/// the peer can complete a handshake and holds the key it presented. It is
/// used only when no `CONNECTOR_TLS_CA_FILE` is configured; the stack's own
/// `Postgres` has one (compose generates its CA).
#[derive(Debug)]
struct AcceptAnyServerCert {
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl ServerCertVerifier for AcceptAnyServerCert {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        // Skipping the certificate CHAIN is the point; the handshake's own
        // signature is still checked, so the peer at least proves possession
        // of the key it presented.
        rustls::crypto::verify_tls12_signature(
            message,
            certificate,
            signature,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            certificate,
            signature,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// The startup-and-authenticate half, generic over any byte stream so it can
/// also be driven over a loopback socket that has no TLS in front of it.
///
/// `password` is the connector's credential, already exposed by the caller
/// for this one computation.
async fn authenticate_and_select<S>(
    stream: &mut S,
    target: &PgDialTarget<'_>,
    password: &str,
) -> Result<(), PgProbeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut out = BytesMut::new();
    frontend::startup_message(
        [("user", target.user), ("database", target.database)],
        &mut out,
    )
    .map_err(|_| PgProbeError::Protocol)?;
    stream.write_all(&out).await?;
    stream.flush().await?;

    let mut buffer = BytesMut::new();
    let mut scram: Option<ScramSha256> = None;
    loop {
        match read_message(stream, &mut buffer).await? {
            Message::AuthenticationOk => break,
            Message::AuthenticationSasl(body) => {
                if !offers_scram_sha256(&body)? {
                    return Err(PgProbeError::NoScram);
                }
                // `unsupported()` is the `n,,` gs2 header: this is
                // SCRAM-SHA-256, not SCRAM-SHA-256-PLUS, so channel binding
                // was never negotiated — what `libpq` sends too.
                let state = ScramSha256::new(password.as_bytes(), ChannelBinding::unsupported());
                let mut out = BytesMut::new();
                frontend::sasl_initial_response(SCRAM_SHA_256, state.message(), &mut out)
                    .map_err(|_| PgProbeError::Protocol)?;
                stream.write_all(&out).await?;
                stream.flush().await?;
                scram = Some(state);
            }
            Message::AuthenticationSaslContinue(body) => {
                let Some(state) = scram.as_mut() else {
                    return Err(PgProbeError::Protocol);
                };
                state
                    .update(body.data())
                    .map_err(|_| PgProbeError::Authentication)?;
                let mut out = BytesMut::new();
                frontend::sasl_response(state.message(), &mut out)
                    .map_err(|_| PgProbeError::Protocol)?;
                stream.write_all(&out).await?;
                stream.flush().await?;
            }
            Message::AuthenticationSaslFinal(body) => {
                let Some(state) = scram.as_mut() else {
                    return Err(PgProbeError::Protocol);
                };
                // Verifies the server's signature against the proof this
                // client just sent: the exchange is checked, not assumed.
                state
                    .finish(body.data())
                    .map_err(|_| PgProbeError::Authentication)?;
            }
            // Every password method other than SCRAM-SHA-256, plus the
            // methods this client has no implementation for: one fixed
            // sentence, and no PasswordMessage is written for any of them.
            Message::AuthenticationCleartextPassword
            | Message::AuthenticationMd5Password(_)
            | Message::AuthenticationKerberosV5
            | Message::AuthenticationGss
            | Message::AuthenticationGssContinue(_)
            | Message::AuthenticationScmCredential
            | Message::AuthenticationSspi => return Err(PgProbeError::RefusedAuthMethod),
            Message::ErrorResponse(_) => return Err(PgProbeError::Authentication),
            _ => return Err(PgProbeError::Protocol),
        }
    }

    let mut out = BytesMut::new();
    frontend::query("SELECT 1", &mut out).map_err(|_| PgProbeError::Protocol)?;
    stream.write_all(&out).await?;
    stream.flush().await?;

    // Read to the end of the query: the server sends CommandComplete and then
    // ReadyForQuery. Requiring the CommandComplete first keeps a
    // startup-time ReadyForQuery (some servers emit one right after
    // authentication) from ending the read before the query ran.
    let mut saw_command_complete = false;
    loop {
        match read_message(stream, &mut buffer).await? {
            Message::CommandComplete(_) => saw_command_complete = true,
            Message::ReadyForQuery(_) if saw_command_complete => break,
            Message::ReadyForQuery(_)
            | Message::RowDescription(_)
            | Message::DataRow(_)
            | Message::ParameterStatus(_)
            // Sent once per session right after `AuthenticationOk`, with the
            // backend's cancellation keys. This probe never cancels, so it
            // reads past it.
            | Message::BackendKeyData(_)
            | Message::EmptyQueryResponse => {}
            Message::ErrorResponse(_) => return Err(PgProbeError::Query),
            _ => return Err(PgProbeError::Protocol),
        }
    }

    // Best-effort clean shutdown; the connection's fate is decided already.
    let mut out = BytesMut::new();
    frontend::terminate(&mut out);
    let _ = stream.write_all(&out).await;
    let _ = stream.flush().await;
    Ok(())
}

/// Does the `AuthenticationSasl` offer include `SCRAM-SHA-256` (not only
/// `SCRAM-SHA-256-PLUS`)?
fn offers_scram_sha256(body: &AuthenticationSaslBody) -> Result<bool, PgProbeError> {
    let mut mechanisms = body.mechanisms();
    loop {
        match mechanisms.next().map_err(|_| PgProbeError::Protocol)? {
            Some(name) => {
                if name == SCRAM_SHA_256 {
                    return Ok(true);
                }
            }
            None => return Ok(false),
        }
    }
}

/// Maximum backend message length accepted by the probe (64 KiB; SEC-14B review round 1, BLOCKER 2).
/// Prevents a rogue server from forcing a ~4 GiB buffer allocation via an untrusted length header.
const MAX_BACKEND_MESSAGE_LEN: usize = 64 * 1024;

/// Read one full backend message, filling `buffer` from `stream` until
/// `Message::parse` can decode it. `Message::parse` consumes exactly one
/// message and leaves any following bytes in `buffer`.
async fn read_message<S>(stream: &mut S, buffer: &mut BytesMut) -> Result<Message, PgProbeError>
where
    S: AsyncRead + Unpin,
{
    loop {
        if buffer.len() >= 5 {
            let len = u32::from_be_bytes([buffer[1], buffer[2], buffer[3], buffer[4]]);
            if len < 4 || (len as usize) > MAX_BACKEND_MESSAGE_LEN {
                return Err(PgProbeError::Protocol);
            }
            if buffer.len() > len as usize {
                if let Some(message) = Message::parse(buffer).map_err(|_| PgProbeError::Protocol)? {
                    return Ok(message);
                }
                return Err(PgProbeError::Protocol);
            }
        }
        let mut chunk = [0u8; 4096];
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            // The server closed before a complete message arrived.
            return Err(PgProbeError::Connection);
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::time::Duration;

    use lakehouse_test_support as _;
    use sqlx::PgPool;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;
    use crate::connector_probe::resolve_checked;
    use crate::internal_hosts::InternalHosts;
    use crate::tls_proxy::{PROXY_SERVER_NAME, TlsProxy, pem_certificate};

    /// A `PgDialTarget` for a loopback test. `host` is the name the TLS
    /// certificate is checked against; the approved address is always
    /// `127.0.0.1`.
    fn target(port: u16, host: &str) -> PgDialTarget<'_> {
        PgDialTarget {
            user: "u",
            host,
            port,
            database: "d",
        }
    }

    async fn approved_for(port: u16) -> Approved {
        resolve_checked("127.0.0.1", port, &InternalHosts::ALL)
            .await
            .expect("127.0.0.1 resolves")
    }

    /// Length-prefixed backend message: `tag`, then the i32 length that
    /// counts itself, then the body.
    fn backend_message(tag: u8, body: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(body.len() + 5);
        out.push(tag);
        let length = i32::try_from(body.len() + 4).unwrap();
        out.extend_from_slice(&length.to_be_bytes());
        out.extend_from_slice(body);
        out
    }

    /// Read one length-prefixed backend/startup message from `stream`,
    /// returning its bytes. A startup message carries no tag, so `tagged`
    /// is false for it.
    async fn read_framed(stream: &mut TcpStream, tagged: bool) -> Vec<u8> {
        let header = if tagged { 5 } else { 4 };
        let mut head = vec![0u8; header];
        stream.read_exact(&mut head).await.unwrap();
        let length_offset = usize::from(tagged);
        let length = i32::from_be_bytes(head[length_offset..length_offset + 4].try_into().unwrap());
        let body = usize::try_from(length).unwrap() - 4;
        let mut rest = vec![0u8; body];
        stream.read_exact(&mut rest).await.unwrap();
        let mut full = head;
        full.extend_from_slice(&rest);
        full
    }

    /// Everything the server received after the message it was waiting for,
    /// or an empty vector when the client sent nothing more before the read
    /// timed out or the socket closed.
    async fn extra_bytes_after(stream: &mut TcpStream) -> Vec<u8> {
        let mut extra = Vec::new();
        let mut chunk = [0u8; 64];
        loop {
            match tokio::time::timeout(Duration::from_millis(300), stream.read(&mut chunk)).await {
                Ok(Ok(0) | Err(_)) | Err(_) => break,
                Ok(Ok(read)) => extra.extend_from_slice(&chunk[..read]),
            }
        }
        extra
    }

    /// (a) A server that answers `SSLRequest` with `N` is refused with the
    /// TLS class, and it never receives a startup packet. No TLS server and
    /// no credential are involved at all, which is the point: the refusal
    /// happens before the startup packet could carry one.
    #[tokio::test]
    async fn a_server_without_tls_is_refused_before_the_startup_packet() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            // SSLRequest: an 8-byte untagged frame.
            let ssl_request = read_framed(&mut socket, false).await;
            assert_eq!(ssl_request.len(), 8, "SSLRequest is 8 bytes");
            socket.write_all(b"N").await.unwrap();
            socket.flush().await.unwrap();
            let extra = extra_bytes_after(&mut socket).await;
            assert!(
                extra.is_empty(),
                "the client must send no startup packet after a refused TLS upgrade"
            );
        });

        let approved = approved_for(port).await;
        let err = connect_and_select(
            &target(port, "localhost"),
            &approved,
            &SecretValue::new("pw"),
            None,
        )
        .await
        .expect_err("a server that refuses TLS must be refused");
        assert_eq!(err.to_string(), "TLS error");
        server.await.unwrap();
    }

    /// (b) `SEC-14-AC2`'s regression test, through the WHOLE path: TLS
    /// first (the proxy terminates it), then a server that completes the
    /// handshake and asks for the password in the clear. The fixed refusal
    /// comes back, and the credential is never written — the server sees no
    /// `PasswordMessage` (`'p'`) after the startup packet.
    ///
    /// Before the fix this test fails: `sqlx` answered an
    /// `AuthenticationCleartextPassword` by sending the password.
    #[tokio::test]
    async fn cleartext_password_is_refused_and_no_password_is_sent() {
        let plain = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let plain_port = plain.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = plain.accept().await.unwrap();
            read_framed(&mut socket, false).await; // startup packet
            // AuthenticationCleartextPassword: 'R', i32 3.
            socket
                .write_all(&backend_message(b'R', &3i32.to_be_bytes()))
                .await
                .unwrap();
            socket.flush().await.unwrap();
            let extra = extra_bytes_after(&mut socket).await;
            assert!(
                extra.is_empty(),
                "no PasswordMessage may follow a cleartext request: {extra:?}"
            );
        });

        let proxy = TlsProxy::start("127.0.0.1", plain_port).await;
        let approved = approved_for(proxy.port()).await;
        let err = connect_and_select(
            &target(proxy.port(), PROXY_SERVER_NAME),
            &approved,
            &SecretValue::new("pw"),
            None,
        )
        .await
        .expect_err("a cleartext request must be refused");
        assert_eq!(
            err.to_string(),
            "server asked for an authentication method this console refuses"
        );
        server.await.unwrap();
    }

    /// (c) The same for `AuthenticationMD5Password`: the same refusal, and no
    /// `PasswordMessage` written for it either.
    #[tokio::test]
    async fn md5_password_is_refused_and_no_password_is_sent() {
        let plain = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let plain_port = plain.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = plain.accept().await.unwrap();
            read_framed(&mut socket, false).await; // startup packet
            // AuthenticationMD5Password: 'R', i32 5, then a 4-byte salt.
            let mut body = 5i32.to_be_bytes().to_vec();
            body.extend_from_slice(&[1, 2, 3, 4]);
            socket
                .write_all(&backend_message(b'R', &body))
                .await
                .unwrap();
            socket.flush().await.unwrap();
            let extra = extra_bytes_after(&mut socket).await;
            assert!(
                extra.is_empty(),
                "no PasswordMessage may follow an MD5 request: {extra:?}"
            );
        });

        let proxy = TlsProxy::start("127.0.0.1", plain_port).await;
        let approved = approved_for(proxy.port()).await;
        let err = connect_and_select(
            &target(proxy.port(), PROXY_SERVER_NAME),
            &approved,
            &SecretValue::new("pw"),
            None,
        )
        .await
        .expect_err("an MD5 request must be refused");
        assert_eq!(
            err.to_string(),
            "server asked for an authentication method this console refuses"
        );
        server.await.unwrap();
    }

    /// (d) A real `SCRAM-SHA-256` exchange against the shared test
    /// `Postgres`, which authenticates with `scram-sha-256` by default: the
    /// whole dial succeeds over an encrypted socket whose certificate is
    /// taken as-is (D5's `require`, `ca` unset).
    #[sqlx::test(migrations = false)]
    async fn scram_against_a_real_server_succeeds(pool: PgPool) {
        let (user, database) = pool_credentials(&pool);
        let proxy = proxy_for(&pool).await;
        connect_and_select(
            &real_target(&user, &database, proxy.port()),
            &approved_for(proxy.port()).await,
            &test_password(),
            None,
        )
        .await
        .expect("SCRAM-SHA-256 over an encrypted socket must succeed");
    }

    /// (d, second half) The same real exchange with `CONNECTOR_TLS_CA_FILE`
    /// set: the chain AND the name are verified against the CA file
    /// (verify-full, D5 as amended).
    #[sqlx::test(migrations = false)]
    async fn verify_full_succeeds_against_the_ca_file(pool: PgPool) {
        let (user, database) = pool_credentials(&pool);
        let proxy = proxy_for(&pool).await;
        let (_ca_dir, ca) = write_ca_file(&proxy);
        connect_and_select(
            &real_target(&user, &database, proxy.port()),
            &approved_for(proxy.port()).await,
            &test_password(),
            Some(ca.as_path()),
        )
        .await
        .expect("verify-full against the CA file must succeed");
    }

    /// (e) With that CA file, a dial whose configured NAME the certificate
    /// does not cover is refused. The refusal is the client's own
    /// verification, before the startup packet, so the server behind the
    /// proxy is never asked for anything.
    #[tokio::test]
    async fn a_ca_file_that_does_not_cover_the_name_refuses_the_dial() {
        // Nothing is ever forwarded to this one: the handshake is refused.
        let plain = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let plain_port = plain.local_addr().unwrap().port();
        let proxy = TlsProxy::start("127.0.0.1", plain_port).await;
        let (_ca_dir, ca) = write_ca_file(&proxy);

        let approved = approved_for(proxy.port()).await;
        let err = connect_and_select(
            &target(proxy.port(), "not-the-server.invalid"),
            &approved,
            &SecretValue::new("pw"),
            Some(ca.as_path()),
        )
        .await
        .expect_err("a name the certificate does not cover must be refused");
        assert_eq!(err.to_string(), "TLS error");
    }

    /// A CA file holding the proxy's own certificate, as
    /// `CONNECTOR_TLS_CA_FILE` would hold an operator's CA. The
    /// [`tempfile::TempDir`] comes back with it so the file outlives the
    /// call and is removed when the test ends.
    fn write_ca_file(proxy: &TlsProxy) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ca.pem");
        std::fs::write(&path, proxy.certificate_pem()).unwrap();
        (dir, path)
    }

    /// A TLS-terminating proxy in front of the server `pool` is connected to.
    async fn proxy_for(pool: &PgPool) -> TlsProxy {
        let options = pool.connect_options();
        TlsProxy::start(options.get_host(), options.get_port()).await
    }

    /// The shared test server's superuser password. `lakehouse-test-support`
    /// always starts the container with the official `Postgres` image's
    /// default one.
    fn test_password() -> SecretValue {
        SecretValue::new("postgres")
    }

    /// A [`PgDialTarget`] for the shared test server, reached through
    /// `proxy`: the name is the one the proxy's certificate carries, so a
    /// verify-full dial has something real to check.
    fn real_target<'a>(user: &'a str, database: &'a str, port: u16) -> PgDialTarget<'a> {
        PgDialTarget {
            user,
            host: PROXY_SERVER_NAME,
            port,
            database,
        }
    }

    /// The user and database of the per-test database `#[sqlx::test]` handed
    /// us, rather than hardcoded ones.
    fn pool_credentials(pool: &PgPool) -> (String, String) {
        let options = pool.connect_options();
        let database = options
            .get_database()
            .expect("#[sqlx::test] always targets a named database")
            .to_owned();
        (options.get_username().to_owned(), database)
    }

    /// `client_config` refuses a CA file it cannot use, rather than failing
    /// every dial at the handshake with a class that does not say why.
    #[test]
    fn an_unusable_ca_file_is_refused_as_a_tls_error() {
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("empty.pem");
        std::fs::write(&empty, b"not a certificate\n").unwrap();
        assert_eq!(
            client_config(Some(empty.as_path()))
                .expect_err("a CA file with no certificate must be refused")
                .to_string(),
            "TLS error"
        );
        assert_eq!(
            client_config(Some(&dir.path().join("absent.pem")))
                .expect_err("a CA file that does not exist must be refused")
                .to_string(),
            "TLS error"
        );
        assert_eq!(
            client_config(None)
                .expect("no CA is D5's require posture, and it is configurable")
                .alpn_protocols,
            Vec::<Vec<u8>>::new()
        );
    }

    /// A certificate that is not a certificate is refused as PEM input, not
    /// accepted as a trust anchor.
    #[test]
    fn a_certificate_that_is_not_one_is_refused() {
        let der = rustls::pki_types::CertificateDer::from(vec![1u8, 2, 3]);
        let pem = pem_certificate(&der);
        assert!(pem.starts_with("-----BEGIN CERTIFICATE-----"));
        let mut roots = RootCertStore::empty();
        let parsed: Result<Vec<_>, _> = CertificateDer::pem_slice_iter(pem.as_bytes())
            .map(|certificate| certificate.map_err(|err| err.to_string()))
            .collect();
        let parsed = parsed.expect("the PEM block itself parses");
        assert_eq!(parsed.len(), 1);
        assert!(
            roots.add(parsed.into_iter().next().unwrap()).is_err(),
            "a root store must not accept arbitrary bytes as a CA"
        );
    }

    /// (f) A backend message header claiming a huge length (e.g. 0xFFFFFFFF)
    /// is refused as a protocol error without growing the buffer capacity (SEC-14B review round 1, BLOCKER 2).
    #[tokio::test]
    async fn oversized_backend_message_length_is_refused_without_buffer_growth() {
        let mut stream = std::io::Cursor::new(b"R\xff\xff\xff\xff");
        let mut buffer = BytesMut::new();
        let Err(err) = read_message(&mut stream, &mut buffer).await else {
            panic!("oversized message length must be refused");
        };
        assert!(matches!(err, PgProbeError::Protocol));
        assert!(
            buffer.capacity() < MAX_BACKEND_MESSAGE_LEN,
            "buffer capacity must not grow to 4 GiB, was {}",
            buffer.capacity()
        );
    }

    /// (f, second half) The same oversized header sent by a fake server over
    /// a full dial is refused as a protocol error.
    #[tokio::test]
    async fn fake_server_sending_huge_length_is_refused() {
        let plain = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let plain_port = plain.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = plain.accept().await.unwrap();
            read_framed(&mut socket, false).await; // startup packet
            socket.write_all(b"R\xff\xff\xff\xff").await.unwrap();
            socket.flush().await.unwrap();
        });

        let proxy = TlsProxy::start("127.0.0.1", plain_port).await;
        let approved = approved_for(proxy.port()).await;
        let err = connect_and_select(
            &target(proxy.port(), PROXY_SERVER_NAME),
            &approved,
            &SecretValue::new("pw"),
            None,
        )
        .await
        .expect_err("oversized length must be refused");
        assert_eq!(
            err.to_string(),
            "the server sent a message this probe did not understand"
        );
        server.await.unwrap();
    }
}
