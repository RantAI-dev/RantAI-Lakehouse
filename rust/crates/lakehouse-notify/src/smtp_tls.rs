//! The TLS half of the `SMTP` sender: the parameters a relay is reached
//! with, and how a failed handshake is told apart from a refused message
//! (`SEC-56`).
//!
//! # Never weaker than before
//!
//! The bundle's CAs are ADDED to the built-in roots (`SEC-56` D2), the
//! hostname check stays on, and there is no `Tls::None` and no
//! `Tls::Opportunistic` anywhere: a relay without TLS is not supported
//! (D3, D4). The two modes are the ones the sender already used —
//! `SMTP_SECURE` in effect means implicit TLS (`Tls::Wrapper`), otherwise
//! `STARTTLS` is REQUIRED (`Tls::Required`), with no plaintext fallback.

use std::error::Error as _;

use lettre::transport::smtp::Error;
use lettre::transport::smtp::client::{Tls, TlsParameters};

use crate::smtp_ca::SmtpCaBundle;

/// The TLS parameters for a relay whose certificate may chain to the
/// bundle's CAs (`SEC-56` D2/D3).
///
/// `TlsParameters::builder` starts from the default root store — the
/// built-in `webpki-roots` — and `add_root_certificate` ADDS to it, so a
/// deployment that trusts a private CA can still reach a public relay (D2).
///
/// The domain is the relay's host: it is the name the certificate must
/// carry, so the hostname check stays on (D3).
///
/// # Errors
///
/// Returns the transport's own error when the parameters cannot be built
/// (a malformed bundle). The caller turns that into
/// [`crate::DeliveryFailure::MailServerTlsFailed`].
pub(crate) fn parameters(host: &str, bundle: &SmtpCaBundle) -> Result<TlsParameters, Error> {
    let mut builder = TlsParameters::builder(host.to_owned());
    for certificate in bundle.certificates() {
        builder = builder.add_root_certificate(certificate.clone());
    }
    builder.build()
}

/// The TLS mode for a relay, given the sender's `SMTP_SECURE` decision.
///
/// `SMTP_SECURE` in effect is implicit TLS (`Tls::Wrapper`); otherwise
/// `STARTTLS` is required (`Tls::Required`) — never opportunistic, never
/// plaintext (`SEC-56` D3).
pub(crate) fn mode(secure: bool, parameters: TlsParameters) -> Tls {
    if secure {
        Tls::Wrapper(parameters)
    } else {
        Tls::Required(parameters)
    }
}

/// Whether the transport failed to make a secure connection
/// (`SEC-56` D7).
///
/// Two shapes reach us, and both are "a secure connection to the mail
/// server could not be made":
///
/// - the TLS configuration could not be built at all — the transport reports
///   that as a TLS error (`lettre-0.11.23/src/transport/smtp/error.rs:76`,
///   `is_tls`);
/// - the handshake itself failed — an untrusted certificate, or a name that
///   is not on the certificate. The transport reports THAT as a *connection*
///   error whose source is the `rustls` error
///   (`lettre-0.11.23/src/transport/smtp/client/async_net.rs:362`,
///   `map_err(error::connection)`), and `tokio-rustls` puts the `rustls`
///   error inside an `io::Error`
///   (`tokio-rustls-0.26.6/src/common/mod.rs:115`).
///
/// The second shape is the one an operator can act on: it is what tells
/// them to look at `SMTP_CA_FILE` and `SMTP_HOST`. Neither the
/// certificate's text nor the server's reply is ever returned (D7,
/// principle 4).
pub(crate) fn is_failure(err: &Error) -> bool {
    if err.is_tls() {
        return true;
    }
    let mut source = err.source();
    while let Some(err) = source {
        if err.downcast_ref::<rustls::Error>().is_some() {
            return true;
        }
        if let Some(io_error) = err.downcast_ref::<std::io::Error>()
            && io_error
                .get_ref()
                .and_then(|inner| inner.downcast_ref::<rustls::Error>())
                .is_some()
        {
            return true;
        }
        source = err.source();
    }
    false
}
