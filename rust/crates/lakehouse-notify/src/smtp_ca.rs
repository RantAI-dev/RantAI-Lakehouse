//! The CA certificates an `SMTP` relay's own certificate may chain to
//! (`SEC-56`).
//!
//! # Why this exists
//!
//! `lettre` is built with the `rustls-tls` feature, which trusts the
//! `webpki-roots` bundle and nothing else: a relay whose certificate chains
//! to a company CA fails the handshake, and every send ends in the same
//! sentence as a wrong host ("the mail server could not be reached"). There
//! was no setting to change that — [`SmtpConfig`](crate::SmtpConfig) had
//! six fields, none about trust.
//!
//! So the operator can hand the sender a bundle of CA certificates
//! (`SMTP_CA_FILE`), and those CAs are ADDED to the built-in roots
//! (`SEC-56` D2): a deployment that trusts a private CA can still reach a
//! public relay. Nothing here weakens the connection — the hostname check
//! stays on, and there is no plaintext mode (D3, D4).
//!
//! # What the type says
//!
//! The bundle holds certificates, not a secret, but the type says what it
//! holds and its `Debug` prints the count only (D9), so
//! [`SmtpConfig`](crate::SmtpConfig) keeps its `derive(Debug)` without
//! printing a PEM.
//!
//! The bundle is read once, at start-up, and kept in memory (D6): the send
//! path does no file I/O.

use std::fmt;

use rustls_pki_types::CertificateDer;
use rustls_pki_types::pem::PemObject;

/// Why a CA bundle could not be read (`SEC-56` D5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaBundleError {
    /// The bytes hold no PEM certificate at all: an empty file, or a file
    /// with something else in it.
    ///
    /// This is the "the operator pointed at the wrong file" case, and it is
    /// a start-up failure, never a silent "trust everything" (D5).
    Empty,
    /// The bytes hold a PEM block that could not be decoded as a
    /// certificate.
    Invalid,
}

impl CaBundleError {
    /// The fixed sentence for this reason; nothing from the file is added
    /// to it.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Empty => "the file holds no PEM certificate",
            Self::Invalid => "the file holds a PEM block that is not a certificate",
        }
    }
}

impl fmt::Display for CaBundleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for CaBundleError {}

/// The CA certificates an `SMTP` relay's certificate may chain to
/// (`SEC-56` D2/D9).
///
/// One bundle holds one or more certificates: the file an operator points
/// `SMTP_CA_FILE` at may be a single CA or a chain of them, and every one
/// of them is added to the transport's root store (D2).
#[derive(Clone, PartialEq, Eq)]
pub struct SmtpCaBundle {
    /// Each certificate's DER, in the order they were read.
    ///
    /// The DER and not the transport's own `Certificate`, because that type
    /// has neither `PartialEq` nor a useful `Debug` (D9): these bytes are
    /// what the transport is given, and what two bundles are compared by.
    der: Vec<Vec<u8>>,
}

impl SmtpCaBundle {
    /// Read a bundle from PEM bytes.
    ///
    /// Every PEM certificate in the bytes is read, so a bundle of several
    /// CAs is one bundle (`SEC-56` T2 case 6).
    ///
    /// # Errors
    ///
    /// - [`CaBundleError::Empty`] when the bytes hold no PEM certificate.
    /// - [`CaBundleError::Invalid`] when a PEM block cannot be decoded.
    pub fn from_pem(pem: &[u8]) -> Result<Self, CaBundleError> {
        let mut der = Vec::new();
        for certificate in CertificateDer::pem_slice_iter(pem) {
            let certificate = certificate.map_err(|_| CaBundleError::Invalid)?;
            der.push(certificate.as_ref().to_vec());
        }
        if der.is_empty() {
            return Err(CaBundleError::Empty);
        }
        Ok(Self { der })
    }

    /// How many certificates the bundle holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.der.len()
    }

    /// Whether the bundle holds no certificate. A bundle read by
    /// [`Self::from_pem`] never is: an empty one is an error, not a bundle.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.der.is_empty()
    }

    /// The certificates' DER, for the transport's root store.
    pub(crate) fn der(&self) -> &[Vec<u8>] {
        &self.der
    }
}

/// The count, and nothing else: a certificate is not a secret, but a `Debug`
/// that printed the PEM would put a blob of base64 into every log line that
/// prints a config (`SEC-56` D9).
impl fmt::Debug for SmtpCaBundle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SmtpCaBundle")
            .field("certificates", &self.der.len())
            .finish()
    }
}

/// Test-only: a certificate minted for the test that needs one.
#[cfg(test)]
pub(crate) mod test_support {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::CertificateDer;

    /// A PEM certificate minted for a test only (`SEC-56` D8: no certificate
    /// and no key is ever committed to this repository — the
    /// `.gitleaksignore` history, `SEC-36`, reads a committed key as a
    /// leak).
    pub(crate) fn minted_certificate(subject_alt_names: &[&str]) -> String {
        let names = subject_alt_names
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>();
        let certified = rcgen::generate_simple_self_signed(names).unwrap();
        pem_certificate(certified.cert.der())
    }

    /// The PEM armour around a DER certificate, written here so `rcgen`'s
    /// `pem` feature stays off (turning it on would unify the feature into
    /// `lakehouse-api`'s tests, which pin `rcgen` without it).
    fn pem_certificate(der: &CertificateDer<'_>) -> String {
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
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::test_support::minted_certificate;
    use super::*;

    /// `SEC-56` D5: a file with no certificate in it is refused, never
    /// treated as "trust everything".
    #[test]
    fn a_bundle_with_no_certificate_in_it_is_refused() {
        assert_eq!(
            SmtpCaBundle::from_pem(b"").unwrap_err(),
            CaBundleError::Empty
        );
        assert_eq!(
            SmtpCaBundle::from_pem(b"this is not a pem certificate\n").unwrap_err(),
            CaBundleError::Empty
        );
    }

    /// A PEM block that cannot be decoded is a different failure from an
    /// empty file: the operator pointed at a file that LOOKS like a
    /// certificate bundle and is broken.
    #[test]
    fn a_bundle_with_a_broken_pem_block_is_refused() {
        let broken =
            b"-----BEGIN CERTIFICATE-----\nnot base64 at all!\n-----END CERTIFICATE-----\n";
        assert_eq!(
            SmtpCaBundle::from_pem(broken).unwrap_err(),
            CaBundleError::Invalid
        );
    }

    #[test]
    fn a_bundle_of_one_certificate_holds_one_certificate() {
        let pem = minted_certificate(&["relay.example.com"]);
        let bundle = SmtpCaBundle::from_pem(pem.as_bytes()).unwrap();
        assert_eq!(bundle.len(), 1);
        assert!(!bundle.is_empty());
    }

    /// `SEC-56` T2 case 6: a bundle may hold several CAs.
    #[test]
    fn a_bundle_of_two_certificates_holds_two_certificates() {
        let mut pem = minted_certificate(&["relay.example.com"]);
        pem.push_str(&minted_certificate(&["other.example.com"]));
        let bundle = SmtpCaBundle::from_pem(pem.as_bytes()).unwrap();
        assert_eq!(bundle.len(), 2);
    }

    /// Two bundles with the same certificates are the same bundle, and two
    /// bundles with different ones are not: the `Config` that carries the
    /// bundle compares by these bytes.
    #[test]
    fn two_bundles_are_compared_by_their_certificates() {
        let pem = minted_certificate(&["relay.example.com"]);
        let other = minted_certificate(&["other.example.com"]);
        assert_eq!(
            SmtpCaBundle::from_pem(pem.as_bytes()).unwrap(),
            SmtpCaBundle::from_pem(pem.as_bytes()).unwrap()
        );
        assert_ne!(
            SmtpCaBundle::from_pem(pem.as_bytes()).unwrap(),
            SmtpCaBundle::from_pem(other.as_bytes()).unwrap()
        );
    }

    /// `SEC-56` D9: the `Debug` says how many certificates, and prints no
    /// PEM.
    #[test]
    fn the_debug_of_a_bundle_prints_the_count_and_no_pem() {
        let pem = minted_certificate(&["relay.example.com"]);
        let bundle = SmtpCaBundle::from_pem(pem.as_bytes()).unwrap();
        let text = format!("{bundle:?}");
        assert!(text.contains('1'), "{text}");
        assert!(!text.contains("BEGIN CERTIFICATE"), "{text}");
        assert!(!text.contains("MII"), "{text}");
    }
}
