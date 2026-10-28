//! PR #192 review BLOCKER 10 and BLOCKER 12: the production `sqlx` connect
//! options (`Require` and `VerifyCa`) must dial over TLS without panicking on
//! `rustls`' provider resolution, and `VerifyCa` must work at the approved
//! **IP address** -- which is what a connector dial does (`SEC-15`).
//!
//! # Why the provider has to be installed (BLOCKER 10)
//!
//! `lakehouse-api` links two `rustls` providers: `ring` (through
//! `sqlx-core`'s `tls-rustls-ring-webpki`) and `aws-lc-rs` (through
//! `object_store`/`hyper-rustls`). `rustls` 0.23 cannot pick a process-level
//! default on its own when both features are present, and it panics with
//! "Could not automatically determine the process-level `CryptoProvider` from
//! Rustls crate features" the first time something asks for one.
//!
//! `sqlx-core` 0.9's `VerifyCa` path asks for one: it builds the server
//! verifier with `WebPkiServerVerifier::builder(...)`
//! (`sqlx-core-0.9.0/src/net/tls/tls_rustls.rs:155`), and that builder reads
//! the process-level default provider. So the API binary installs `ring`
//! explicitly at the start of `main`
//! (`lakehouse_api::crypto_provider::install_default_provider`), and this test
//! binary calls the SAME function, so a regression in either is visible here.
//!
//! # Why the `VerifyCa` dial is at an IP (BLOCKER 12)
//!
//! A connector dial goes to the approved address, never the configured name
//! (`SEC-15`), so the certificate's name can never match the dialed host and
//! the name check must be skipped -- that is what `VerifyCa` means.
//! `sqlx-core` 0.8.6 only recognised `rustls`' `NotValidForName`, while
//! `rustls` 0.23.46 reports `NotValidForNameContext`, so every `VerifyCa`
//! dial to an IP failed (`SEC-41`'s "Why"). `sqlx-core` 0.9.0 recognises both
//! (`sqlx-core-0.9.0/src/net/tls/tls_rustls.rs:313`). The test below pins
//! that half down through the PRODUCTION helper
//! ([`pg_options_for_address`]), so the fix cannot be undone in this branch's
//! own code.
//!
//! The TLS terminator in front of the plain test `Postgres` is the shared
//! [`common::tls_proxy::TlsProxy`] (`src/tls_proxy.rs`, included by
//! `tests/common/mod.rs`), not a second copy: one implementation, used by
//! every test that needs a `Postgres` that speaks TLS.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Write;

use lakehouse_api::connector_probe::pg_options_for_address;
use lakehouse_api::crypto_provider::install_default_provider;
use sqlx::{Connection, PgConnection};

// Force-links lakehouse-test-support for the shared testcontainer Postgres.
use lakehouse_test_support as _;

mod common;

/// The address a connector dial really uses: the approved IP, never the name.
const APPROVED_ADDRESS: &str = "127.0.0.1";

fn parse_postgres_url(url: &str) -> (String, String, String, u16) {
    let rest = url
        .strip_prefix("postgres://")
        .expect("lakehouse_test_support::database_url is a postgres:// URL");
    let (auth, host_port_db) = rest.split_once('@').expect("user:pass@host:port/db");
    let (user, password) = auth.split_once(':').unwrap_or((auth, ""));
    let (host_port, _db) = host_port_db.split_once('/').unwrap_or((host_port_db, ""));
    let (host, port) = host_port.rsplit_once(':').expect("host:port");
    (
        user.to_owned(),
        password.to_owned(),
        host.to_owned(),
        port.parse().expect("port integer"),
    )
}

/// The production helper's no-CA posture (`D5`/`E1`): encrypt, and take the
/// server certificate as-is.
#[tokio::test]
async fn sqlx_require_dial_at_the_approved_address_succeeds_with_the_installed_provider() {
    // Install the ring provider using the exact same function main() calls.
    install_default_provider().expect("install ring crypto provider");

    let url = lakehouse_test_support::database_url();
    let (user, password, host, port) = parse_postgres_url(&url);
    let proxy = common::tls_proxy::TlsProxy::start(&host, port).await;

    let require_opts = pg_options_for_address(
        APPROVED_ADDRESS,
        proxy.port(),
        &user,
        &password,
        "postgres",
        None,
    );
    let mut conn = PgConnection::connect_with(&require_opts)
        .await
        .expect("sqlx connect with Require ssl mode over tls_proxy");
    let row: (i32,) = sqlx::query_as("SELECT 1")
        .fetch_one(&mut conn)
        .await
        .expect("query SELECT 1 over Require TLS");
    assert_eq!(row.0, 1);
    drop(conn);
}

/// The production helper's CA posture at the approved **IP address**:
/// `VerifyCa` proves the chain and skips the name check (`D5`/`E1`,
/// `SEC-15`). This is BLOCKER 12's regression test: with `sqlx` 0.8.6 the
/// dial failed on `rustls`' `NotValidForNameContext`.
#[tokio::test]
async fn sqlx_verify_ca_dial_at_the_approved_address_succeeds_with_the_installed_provider() {
    // Install the ring provider using the exact same function main() calls.
    install_default_provider().expect("install ring crypto provider");

    let url = lakehouse_test_support::database_url();
    let (user, password, host, port) = parse_postgres_url(&url);
    let proxy = common::tls_proxy::TlsProxy::start(&host, port).await;

    // The proxy's own certificate is the CA here: it is self-signed, and it
    // names `localhost` -- NOT the IP this test dials. So the chain verifies
    // and the name check is the only thing that cannot pass, which is
    // exactly what `VerifyCa` is for.
    let mut temp_ca = tempfile::NamedTempFile::new().expect("create temp CA file");
    temp_ca
        .write_all(proxy.certificate_pem().as_bytes())
        .expect("write proxy certificate to temp CA file");
    temp_ca.flush().expect("flush temp CA file");

    let verify_ca_opts = pg_options_for_address(
        APPROVED_ADDRESS,
        proxy.port(),
        &user,
        &password,
        "postgres",
        Some(temp_ca.path()),
    );
    let mut conn = PgConnection::connect_with(&verify_ca_opts)
        .await
        .expect("sqlx connect with VerifyCa ssl mode over tls_proxy");
    let row: (i32,) = sqlx::query_as("SELECT 1")
        .fetch_one(&mut conn)
        .await
        .expect("query SELECT 1 over VerifyCa TLS");
    assert_eq!(row.0, 1);
    drop(conn);
}
