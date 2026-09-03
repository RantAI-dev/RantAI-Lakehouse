//! G1 acceptance test — the P1 "floor" gate from
//! `docs/plans/LAKEHOUSE-FOUNDATION-PLAN.md` §3.
//!
//! Two halves, both required:
//!
//! - **(a)** This crate creates a Bronze Iceberg table through Lakekeeper
//!   and appends rows to it, using vended S3 credentials; `ClickHouse`
//!   reads those rows back through a `DataLakeCatalog` database.
//! - **(b)** `ClickHouse` `CREATE TABLE` + `INSERT` **through the
//!   catalog** (not path-based `IcebergS3`); the result is read back via
//!   this crate (`iceberg-rust`).
//!
//! # This test needs a live stack — it does not spawn one
//!
//! Same convention `lakehouse-api/tests/parity.rs` already uses for
//! stack-dependent tests: `#[ignore]`d by default (so a plain
//! `cargo test --all-features` never needs Docker), run explicitly against
//! a running `docker compose` stack:
//!
//! ```bash
//! docker compose -p p1bcheck up -d --build
//! # wait for lakekeeper, rustfs, clickhouse healthy
//! LAKEKEEPER_CATALOG_URI=http://localhost:8181/catalog \
//! LAKEKEEPER_WAREHOUSE=default \
//! CH_LAKEKEEPER_CATALOG_URI=http://lakekeeper:8181/catalog \
//! CH_RUSTFS_S3_ENDPOINT=http://rustfs:9000 \
//! CH_URL=http://localhost:8123 \
//! CH_USER=default \
//! CH_PASSWORD= \
//!   cargo test -p lakehouse-iceberg --test g1_lakekeeper -- --ignored --nocapture
//! ```
//!
//! `LAKEKEEPER_CATALOG_URI` is host-reachable (this test process's own
//! view); `CH_LAKEKEEPER_CATALOG_URI`/`CH_RUSTFS_S3_ENDPOINT` are reachable
//! from INSIDE the compose network, because that is where the `clickhouse`
//! container itself resolves `lakekeeper`/`rustfs` — see [`G1Env`].
//!
//! The `lakekeeper-warehouse-init` compose service (see
//! `docker-compose.yml`) bootstraps the Lakekeeper server and creates the
//! `LAKEKEEPER_WAREHOUSE` warehouse automatically on `docker compose up`;
//! nothing needs to be curled by hand.
//!
//! **Running `cargo test` on the host, not in a container on the compose
//! network:** Lakekeeper vends this crate the S3 endpoint it knows the
//! object store by — `http://rustfs:9000`, the compose service DNS name —
//! regardless of who's asking. A test process running directly on the
//! host (as opposed to inside a container on `p1bcheck_default`) cannot
//! resolve `rustfs` unless something makes it resolvable, e.g. an
//! `/etc/hosts` entry pointing at the `rustfs` container's bridge-network
//! IP (`docker inspect <project>-rustfs-1 --format
//! '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}'`) — that
//! bridge IP is reachable directly from the host without going through
//! the container's host-port mapping, since the mapped port differs. CI
//! should instead run this test itself inside a container attached to the
//! compose network, avoiding the workaround entirely.
//!
//! # Half (b) status: currently blocked on a `ClickHouse` crash, not Lakekeeper
//!
//! As of `ClickHouse` 26.3.26.3, half (b) does not pass — but not because of
//! Lakekeeper. See the P1b report for the full finding: `CREATE TABLE`
//! without an explicit `ENGINE` inside a `DataLakeCatalog` database never
//! reaches Lakekeeper at all (it silently falls back to a default
//! `MergeTree` table and fails), and `INSERT INTO` an *existing*
//! catalog-registered table (one this crate created) segfaults the
//! server inside `DB::IcebergStorageSink::consume` /
//! `DB::ChunkPartitioner::partitionChunk`. Per the task brief's stop
//! condition, this is reported rather than worked around with a
//! path-based `IcebergS3` fallback — the two `#[ignore]`d tests below are
//! written to the spec regardless, so they start passing the moment
//! `ClickHouse`'s write path is fixed.
//!
//! # Why this actually proves vended credentials, not just connectivity
//!
//! Half (a)'s `object_store` client is never handed `RUSTFS_ACCESS_KEY`/
//! `RUSTFS_SECRET_KEY` anywhere in this test or in `lakehouse-iceberg`
//! itself — see `catalog.rs`'s and `storage.rs`'s module docs. If
//! Lakekeeper were not vending credentials (e.g. the
//! `X-Iceberg-Access-Delegation` header were dropped, or Lakekeeper's
//! storage-credential/STS wiring were broken), this test fails at the
//! `append` call with an S3 authentication error — it does not fall back
//! to anything static.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use arrow_array::{ArrayRef, Int64Array, RecordBatch, StringArray, TimestampMicrosecondArray};
use iceberg::spec::{NestedField, PrimitiveType, Schema as IcebergSchema, Type};
use lakehouse_iceberg::catalog::{IcebergClient, IcebergClientConfig};

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| default.to_owned())
}

struct G1Env {
    /// Lakekeeper catalog URI as reached from THIS test process (the host,
    /// or wherever `cargo test` runs) — used by `lakehouse-iceberg`.
    catalog_uri: String,
    /// Lakekeeper catalog URI as reached from INSIDE the compose network,
    /// i.e. as `ClickHouse`'s own container sees it. Deliberately a
    /// separate field from `catalog_uri`: this test process and the
    /// `clickhouse` container are on different sides of the compose
    /// network boundary and generally cannot use the same hostname/port to
    /// reach the same service (host-mapped port vs. compose service DNS
    /// name), so collapsing these into one field would silently work only
    /// when the two happen to coincide.
    ch_catalog_uri: String,
    /// `RustFS` S3 endpoint as reached from INSIDE the compose network (used
    /// only in the `storage_endpoint` `ClickHouse` DDL setting — the
    /// endpoint this test process itself uses for anything is resolved
    /// internally by `lakehouse-iceberg` from vended credentials, not from
    /// a field on this struct).
    ch_rustfs_s3_endpoint: String,
    warehouse: String,
    ch_url: String,
    ch_user: String,
    ch_password: String,
    /// R1 (ADR 0011): this crate's own bearer token — the `rust-iceberg`
    /// principal, granted `create`/`modify`/`select` on the warehouse.
    /// Empty string when unset (`IcebergClientConfig::catalog_token`
    /// treats an empty token the same as `None` via `Self::token_or_none`
    /// below), which is what a pre-R1 or authz-disabled stack looks like.
    catalog_token: String,
    /// R1 (ADR 0011): the `clickhouse-reader` principal's token, granted
    /// `select` only. Used with `oauth_server_uri` pointed at
    /// `ops/oidc-mock`'s `/token` endpoint — `ClickHouse`'s
    /// `catalog_credential` setting only accepts the Iceberg REST spec's
    /// `client_id:client_secret` `OAuth2` form (measured empirically; see
    /// ADR 0011), never a raw static token the way this crate's own
    /// `token` property does.
    ch_oauth_client_id: String,
    ch_oauth_server_uri: String,
    /// R1 (ADR 0011): the negative-test principal — self-registered with
    /// Lakekeeper but never granted any relation on the warehouse.
    unauthorized_token: String,
}

impl G1Env {
    fn from_process_env() -> Self {
        Self {
            catalog_uri: env_or("LAKEKEEPER_CATALOG_URI", "http://localhost:8181/catalog"),
            ch_catalog_uri: env_or(
                "CH_LAKEKEEPER_CATALOG_URI",
                "http://lakekeeper:8181/catalog",
            ),
            ch_rustfs_s3_endpoint: env_or("CH_RUSTFS_S3_ENDPOINT", "http://rustfs:9000"),
            warehouse: env_or("LAKEKEEPER_WAREHOUSE", "default"),
            ch_url: env_or("CH_URL", "http://localhost:8123"),
            ch_user: env_or("CH_USER", "default"),
            ch_password: env_or("CH_PASSWORD", ""),
            catalog_token: env_or("LAKEKEEPER_TOKEN", ""),
            ch_oauth_client_id: env_or("CH_OAUTH_CLIENT_ID", "clickhouse-reader"),
            ch_oauth_server_uri: env_or("CH_OAUTH_SERVER_URI", "http://oidc-mock:8090/token"),
            unauthorized_token: env_or("LAKEKEEPER_UNAUTHORIZED_TOKEN", ""),
        }
    }

    /// `IcebergClientConfig::catalog_token` wants `None`, not `Some("")`,
    /// on a stack that isn't running R1's authorization at all.
    fn token_or_none(token: &str) -> Option<lakehouse_core::secret::SecretValue> {
        if token.trim().is_empty() {
            None
        } else {
            Some(lakehouse_core::secret::SecretValue::new(token.to_owned()))
        }
    }
}

/// Minimal direct HTTP call against `ClickHouse`'s plain interface, kept
/// local to this test rather than pulling in `lakehouse-clickhouse` as a
/// dependency of the whole crate (only this integration test needs it).
async fn ch_query(env: &G1Env, sql: &str) -> String {
    let client = reqwest::Client::new();
    let response = client
        .post(&env.ch_url)
        .basic_auth(&env.ch_user, Some(&env.ch_password))
        .header("Content-Type", "text/plain; charset=utf-8")
        .body(sql.to_owned())
        .send()
        .await
        .unwrap_or_else(|e| panic!("ClickHouse request failed: {e}\nSQL: {sql}"));
    let status = response.status();
    let text = response.text().await.unwrap();
    assert!(
        status.is_success(),
        "ClickHouse returned {status}: {text}\nSQL: {sql}"
    );
    text
}

/// Domain schema shared by both halves: `id Long`, `label String`.
fn domain_fields() -> Vec<NestedField> {
    vec![
        NestedField::required(
            lakehouse_iceberg::bronze::FIRST_DOMAIN_FIELD_ID,
            "id",
            Type::Primitive(PrimitiveType::Long),
        ),
        NestedField::required(
            lakehouse_iceberg::bronze::FIRST_DOMAIN_FIELD_ID + 1,
            "label",
            Type::Primitive(PrimitiveType::String),
        ),
    ]
}

/// Builds a `RecordBatch` whose Arrow schema is derived from the Iceberg
/// schema via `iceberg::arrow::schema_to_arrow_schema` (rather than a
/// hand-written `arrow_schema::Schema`), because `iceberg-rust`'s Parquet
/// writer maps columns to the Iceberg schema by the `PARQUET:field_id`
/// metadata that conversion attaches to each Arrow field — a hand-written
/// schema without that metadata fails the write with
/// `DataInvalid => Field id N not found in struct array`.
fn test_batch(
    iceberg_schema: &IcebergSchema,
    ingested_at_micros: i64,
    rows: &[(i64, &str)],
) -> RecordBatch {
    let schema = Arc::new(
        iceberg::arrow::schema_to_arrow_schema(iceberg_schema)
            .expect("iceberg schema converts to an arrow schema"),
    );
    let ingested_at: ArrayRef = Arc::new(TimestampMicrosecondArray::from(vec![
        ingested_at_micros;
        rows.len()
    ]));
    let ids: ArrayRef = Arc::new(rows.iter().map(|(id, _)| *id).collect::<Int64Array>());
    let labels: ArrayRef = Arc::new(StringArray::from_iter_values(
        rows.iter().map(|(_, label)| *label),
    ));
    RecordBatch::try_new(schema, vec![ingested_at, ids, labels]).unwrap()
}

/// Half (a): Rust creates the table + appends; `ClickHouse` reads it back
/// through a `DataLakeCatalog` database.
#[tokio::test]
#[ignore = "needs a live docker compose stack; see this file's module doc"]
async fn g1_half_a_rust_writes_clickhouse_reads() {
    let env = G1Env::from_process_env();
    let table_name = "g1_rust_write";

    let client = IcebergClient::connect(&IcebergClientConfig {
        catalog_uri: env.catalog_uri.clone(),
        warehouse: env.warehouse.clone(),
        catalog_credential: None,
        catalog_token: G1Env::token_or_none(&env.catalog_token),
    })
    .await
    .expect("connect to Lakekeeper");

    // Idempotent for repeated local runs: drop-and-recreate isn't
    // available on this crate's API surface (append-only, per the crate
    // doc), so load-or-create instead.
    let mut table = match client.load_bronze_table(table_name).await {
        Ok(t) => t,
        Err(_) => client
            .create_bronze_table(table_name, domain_fields())
            .await
            .expect("create Bronze table"),
    };

    assert_eq!(
        table.format_version(),
        iceberg::spec::FormatVersion::V2,
        "G1 verification: table must be format-version 2"
    );

    let batch = test_batch(
        table.schema(),
        1_735_000_000_000_000,
        &[(1, "alpha"), (2, "beta")],
    );
    let expected_rows = batch.num_rows();
    table
        .append(client.as_catalog(), batch)
        .await
        .expect("append using Lakekeeper-vended credentials");

    // ClickHouse side: a DataLakeCatalog database over the same Lakekeeper
    // warehouse, reading the table Rust just wrote. R1 (ADR 0011): when
    // Lakekeeper authorization is enforced, ClickHouse authenticates as
    // the `clickhouse-reader` principal (`select`-only) via the Iceberg
    // REST spec's OAuth2 client-credentials form — `catalog_credential`
    // does not accept a raw bearer token here (measured; see ADR 0011),
    // so `oauth_server_uri` points at `ops/oidc-mock`'s `/token` endpoint
    // instead of Lakekeeper's own (unverified) `/v1/oauth/tokens`.
    let ch_db = "g1_lakekeeper_a";
    let ch_auth_settings = if env.catalog_token.trim().is_empty() {
        String::new()
    } else {
        format!(
            ", catalog_credential = '{client_id}:unused', oauth_server_uri = '{oauth_uri}'",
            client_id = env.ch_oauth_client_id,
            oauth_uri = env.ch_oauth_server_uri,
        )
    };
    let create_db_sql = format!(
        "CREATE DATABASE IF NOT EXISTS {ch_db} \
         ENGINE = DataLakeCatalog('{catalog_uri}') \
         SETTINGS catalog_type = 'rest', warehouse = '{warehouse}', \
         storage_endpoint = '{storage_endpoint}'{ch_auth_settings} \
         SETTINGS allow_database_iceberg = 1",
        catalog_uri = env.ch_catalog_uri,
        warehouse = env.warehouse,
        storage_endpoint = env.ch_rustfs_s3_endpoint,
    );
    ch_query(&env, &create_db_sql).await;

    // Never a bare, unqualified row-count query against a Bronze Iceberg
    // table: ClickHouse 26.3 takes a metadata-only fast path that
    // overcounts once equality deletes are present (P5-RESULT.md; R11 in
    // the risk register, enforced by the ops/lint R11 guard script). This
    // table is append-only today, but the WHERE-qualified form is the
    // only sanctioned shape.
    let select_sql = format!(
        "SELECT count() FROM {ch_db}.`bronze.{table_name}` WHERE 1 SETTINGS allow_database_iceberg = 1 FORMAT TabSeparated"
    );
    let count_text = ch_query(&env, &select_sql).await;
    let count: usize = count_text
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("expected a row count from ClickHouse, got: {count_text:?}"));
    assert!(
        count >= expected_rows,
        "expected ClickHouse to see at least {expected_rows} rows via DataLakeCatalog, saw {count}"
    );
}

/// Half (b): `ClickHouse` creates the table and inserts through the
/// catalog (not path-based); Rust reads it back via `iceberg-rust`.
///
/// **STOP CONDITION**: per the task brief, if this fails specifically
/// because of Lakekeeper (its authz on metadata updates, its REST surface,
/// a protocol mismatch), this test must be left failing and reported, NOT
/// worked around with a path-based `IcebergS3` fallback. See the P1b
/// report for the outcome of this specific test.
#[tokio::test]
#[ignore = "needs a live docker compose stack; see this file's module doc"]
async fn g1_half_b_clickhouse_writes_rust_reads() {
    let env = G1Env::from_process_env();
    let table_name = "g1_ch_write";
    let ch_db = "g1_lakekeeper_b";

    let create_db_sql = format!(
        "CREATE DATABASE IF NOT EXISTS {ch_db} \
         ENGINE = DataLakeCatalog('{catalog_uri}') \
         SETTINGS catalog_type = 'rest', warehouse = '{warehouse}', \
         storage_endpoint = '{storage_endpoint}' \
         SETTINGS allow_database_iceberg = 1",
        catalog_uri = env.ch_catalog_uri,
        warehouse = env.warehouse,
        storage_endpoint = env.ch_rustfs_s3_endpoint,
    );
    ch_query(&env, &create_db_sql).await;

    let create_table_sql = format!(
        "CREATE TABLE IF NOT EXISTS {ch_db}.`bronze.{table_name}` \
         (id Int64, label String) \
         SETTINGS allow_database_iceberg = 1"
    );
    ch_query(&env, &create_table_sql).await;

    let insert_sql = format!(
        "INSERT INTO {ch_db}.`bronze.{table_name}` (id, label) VALUES \
         (1, 'ch-alpha'), (2, 'ch-beta') \
         SETTINGS allow_insert_into_iceberg = 1, allow_experimental_insert_into_iceberg = 1"
    );
    ch_query(&env, &insert_sql).await;

    // Read back via this crate / iceberg-rust — the whole point of half
    // (b) is that a non-ClickHouse Iceberg reader can see what ClickHouse
    // wrote through the catalog.
    let client = IcebergClient::connect(&IcebergClientConfig {
        catalog_uri: env.catalog_uri.clone(),
        warehouse: env.warehouse.clone(),
        catalog_credential: None,
        catalog_token: G1Env::token_or_none(&env.catalog_token),
    })
    .await
    .expect("connect to Lakekeeper");

    let table = client
        .load_bronze_table(table_name)
        .await
        .expect("load table ClickHouse created through the catalog");

    assert_eq!(
        table.format_version(),
        iceberg::spec::FormatVersion::V2,
        "G1 verification: ClickHouse-created table must also be format-version 2"
    );

    let batches = table.read_all().await.expect("read back via iceberg-rust");
    let total_rows: usize = batches.iter().map(RecordBatch::num_rows).sum();
    assert!(
        total_rows >= 2,
        "expected at least 2 rows written by ClickHouse, read back {total_rows}"
    );

    let mut seen_ids: Vec<i64> = Vec::new();
    for batch in &batches {
        let id_col = batch
            .column_by_name("id")
            .expect("id column present")
            .as_any()
            .downcast_ref::<Int64Array>()
            .expect("id column is Int64");
        seen_ids.extend(id_col.iter().flatten());
    }
    seen_ids.sort_unstable();
    assert!(
        seen_ids.contains(&1) && seen_ids.contains(&2),
        "seen_ids = {seen_ids:?}"
    );
}

/// R1 (ADR 0011) negative test: a principal with NO grant on the
/// warehouse must be **denied**, proving enforcement is real rather than
/// merely configured. `unauthorized-test` (see `ops/oidc-mock/server.py`'s
/// `PRINCIPALS`) is self-registered with Lakekeeper by
/// `lakekeeper-authz-init` — Lakekeeper knows who it is — but never
/// receives a `POST /management/v1/permissions/warehouse/{id}/assignments`
/// grant. Skipped (not `#[ignore]`d silently-pass) when
/// `LAKEKEEPER_UNAUTHORIZED_TOKEN` is unset, i.e. against a pre-R1 or
/// authz-disabled stack, where this scenario does not apply.
#[tokio::test]
#[ignore = "needs a live docker compose stack with R1 authorization enabled; see this file's module doc"]
async fn g1_negative_ungranted_principal_is_denied() {
    let env = G1Env::from_process_env();
    assert!(
        !env.unauthorized_token.trim().is_empty(),
        "LAKEKEEPER_UNAUTHORIZED_TOKEN must be set to run this test — it only applies \
         to a stack with R1 authorization enabled"
    );

    let client = IcebergClient::connect(&IcebergClientConfig {
        catalog_uri: env.catalog_uri.clone(),
        warehouse: env.warehouse.clone(),
        catalog_credential: None,
        catalog_token: G1Env::token_or_none(&env.unauthorized_token),
    })
    .await
    .expect("connect to Lakekeeper (connecting itself does not touch the warehouse)");

    // Any metadata write is denied — `ensure_bronze_namespace` is the
    // cheapest one this crate exposes. Capture and print the ACTUAL
    // denial error, not just assert failure, per the task's own
    // instruction that this is the headline proof.
    let result = client.ensure_bronze_namespace().await;
    let err = result.expect_err(
        "expected the ungranted `unauthorized-test` principal to be DENIED creating a \
         namespace, but the call succeeded — authorization is not actually enforced",
    );
    println!("g1_negative: captured denial error: {err}");
    let message = err.to_string();
    assert!(
        message.contains("403")
            || message.contains("404")
            || message.to_lowercase().contains("forbidden")
            || message.to_lowercase().contains("not exist")
            || message.to_lowercase().contains("not found"),
        "expected an authorization-denial-shaped error (403/404/forbidden/not found), got: {message}"
    );
}
