//! PR #192 review BLOCKER 10: verify that installing the `ring` crypto provider
//! allows production `sqlx` connect options (`Require` and `VerifyCa`) to dial
//! over TLS without panicking on provider resolution.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Write;

use lakehouse_api::connector_probe::pg_options_for_address;
use lakehouse_api::crypto_provider::install_default_provider;
use sqlx::{Connection, PgConnection};

// Force-links lakehouse-test-support for the shared testcontainer Postgres.
use lakehouse_test_support as _;

mod common;

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

#[tokio::test]
async fn sqlx_tls_dial_succeeds_with_installed_crypto_provider() {
    // Install the ring provider using the exact same function main() calls.
    install_default_provider().expect("install ring crypto provider");

    let url = lakehouse_test_support::database_url();
    let (user, password, host, port) = parse_postgres_url(&url);
    let proxy = common::tls_proxy::TlsProxy::start(&host, port).await;

    // 1. Dial with Require (no CA specified)
    let require_opts = pg_options_for_address(
        "127.0.0.1",
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

    // 2. Dial with VerifyCa (CA certificate pointing to proxy's certificate)
    let mut temp_ca = tempfile::NamedTempFile::new().expect("create temp CA file");
    temp_ca
        .write_all(proxy.certificate_pem().as_bytes())
        .expect("write proxy certificate to temp CA file");
    temp_ca.flush().expect("flush temp CA file");

    let verify_ca_opts = pg_options_for_address(
        "localhost",
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
}
