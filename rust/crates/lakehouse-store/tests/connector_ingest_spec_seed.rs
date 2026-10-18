//! Integration tests for `0034_seed_connector_ingest_spec.sql` against a
//! real Postgres.
//!
//! # Postgres backing
//!
//! These are `#[sqlx::test(migrations = "../../migrations")]` tests: each
//! one gets a freshly migrated, isolated database. The Postgres *server*
//! itself is started once per test binary by the `lakehouse-test-support`
//! dev-dependency, which spins up a `testcontainers`-managed Postgres and
//! points `DATABASE_URL` at it before any test runs — no manual
//! `docker compose up`, no external database required. Docker must be
//! reachable from the environment running `cargo test`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap actually runs for this test binary (an
// unreferenced dev-dependency's rlib member can otherwise be dropped
// by the linker before its ctor section is ever considered).
use lakehouse_test_support as _;

use lakehouse_store::connectors::get_ingest_spec;
use lakehouse_store::ingest_spec::Dial;
use sqlx::PgPool;

/// `0034` writes `"driver":"postgres"` (not `"postgresql"`) into
/// `conn-pg-lakehouse`'s seeded `dial`, because `SqlDriver`
/// (`lakehouse-store/src/ingest_spec.rs`) is
/// `#[serde(rename_all = "snake_case")]` over `Mysql | Postgres | Mssql`.
/// The migration writes straight to the database, bypassing
/// `set_ingest_spec`'s `Dial::parse` guard, so this test parses the seeded
/// row's `dial` through the same `Dial::parse` the application uses, to
/// pin the seed against ever drifting from the enum again.
#[sqlx::test(migrations = "../../migrations")]
async fn seeded_pg_lakehouse_dial_parses_as_postgres(pool: PgPool) -> sqlx::Result<()> {
    let spec = get_ingest_spec(&pool, "conn-pg-lakehouse")
        .await
        .expect("get_ingest_spec should succeed")
        .expect("conn-pg-lakehouse should exist");
    let adapter = spec.adapter.as_deref().expect("adapter should be seeded");
    assert_eq!(adapter, "sql");

    let dial = Dial::parse(adapter, &spec.dial).expect("seeded dial should parse");
    match dial {
        Dial::Sql(sql) => {
            assert_eq!(
                sql.user, "lakehouse",
                "dial.user must be a literal, never env:-prefixed"
            );
        }
        other => panic!("expected Dial::Sql, got {other:?}"),
    }
    Ok(())
}

/// `conn-s3-warehouse`'s `source_objects` is seeded empty — no tracked file
/// creates a `landing/` prefix or CSV fixture on the warehouse bucket to
/// describe (see `0034_seed_connector_ingest_spec.sql`'s header comment).
#[sqlx::test(migrations = "../../migrations")]
async fn seeded_s3_warehouse_has_empty_source_objects(pool: PgPool) -> sqlx::Result<()> {
    let spec = get_ingest_spec(&pool, "conn-s3-warehouse")
        .await
        .expect("get_ingest_spec should succeed")
        .expect("conn-s3-warehouse should exist");
    assert_eq!(spec.adapter.as_deref(), Some("files"));
    assert_eq!(spec.source_objects, serde_json::json!([]));

    let adapter = spec.adapter.as_deref().expect("adapter should be seeded");
    Dial::parse(adapter, &spec.dial).expect("seeded files dial should parse");
    Ok(())
}
