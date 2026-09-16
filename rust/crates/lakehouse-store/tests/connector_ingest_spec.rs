//! Integration tests for `0033_connector_ingest_spec.sql` against a real
//! Postgres.
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

use lakehouse_store::connectors::{CreateConnectorInput, create_connector, get_ingest_spec};
use sqlx::PgPool;

/// `0033_connector_ingest_spec.sql` grants `ingest:read` to the seeded Data
/// Engineer role so `GET /api/connectors/{id}/ingest-spec` can be gated on
/// a narrow, real permission (WS3 plan review X7) instead of reusing
/// `connector:manage`.
#[sqlx::test(migrations = "../../migrations")]
async fn data_engineer_role_gains_ingest_read(pool: PgPool) -> sqlx::Result<()> {
    let (perms,): (String,) =
        sqlx::query_as("SELECT permissions FROM role WHERE name = 'Data Engineer'")
            .fetch_one(&pool)
            .await?;
    assert!(
        perms.contains("ingest:read"),
        "expected Data Engineer's permissions to contain ingest:read, got: {perms}"
    );
    Ok(())
}

/// `0033`'s `UPDATE` is keyed on `ingest:read`'s absence (following
/// `0020_extend_role_grants.sql`'s guard shape), not an exact-value match
/// on the whole permission string, so it never double-appends the token
/// even though the migration chain applies once per fresh test database.
/// This pins down the observable effect of that guard: exactly one
/// `ingest:read` token in the final string.
#[sqlx::test(migrations = "../../migrations")]
async fn ingest_read_is_granted_exactly_once(pool: PgPool) -> sqlx::Result<()> {
    let (perms,): (String,) =
        sqlx::query_as("SELECT permissions FROM role WHERE name = 'Data Engineer'")
            .fetch_one(&pool)
            .await?;
    let occurrences = perms.matches("ingest:read").count();
    assert_eq!(
        occurrences, 1,
        "expected ingest:read exactly once in Data Engineer's permissions, got: {perms}"
    );
    Ok(())
}

/// The five additive columns from `0033` exist with the documented
/// defaults, so a connector row that has never had an ingest spec set
/// keeps parsing with `adapter = NULL` ("not ingestible yet") rather than
/// failing to migrate.
///
/// This test previously queried `WHERE id NOT IN ('conn-pg-lakehouse',
/// 'conn-s3-warehouse')` over the seeded rows and looped over the result.
/// After the migration chain that query returns zero rows:
/// `0014_seed_connectors.sql` inserts 28 connector rows and
/// `0022_prune_connector_seed.sql` deletes exactly those 28 and
/// re-inserts only `conn-pg-lakehouse`/`conn-s3-warehouse` — the two IDs
/// the `NOT IN` excludes. So the loop body never ran and the test asserted
/// nothing (WS3 item 10). There is no third seeded row left to observe the
/// defaults on, so this test now creates its own connector row (which
/// `0034_seed_connector_ingest_spec.sql` never touches, since it only
/// updates the two dialable rows by id) and reads the defaults off that
/// via `get_ingest_spec`, the same store function
/// `tests/connectors.rs`'s ingest-spec tests use.
#[sqlx::test(migrations = "../../migrations")]
async fn a_freshly_created_connector_has_null_adapter_and_empty_dial(
    pool: PgPool,
) -> sqlx::Result<()> {
    let created = create_connector(
        &pool,
        &CreateConnectorInput {
            name: "ingest spec column defaults".to_owned(),
            kind: "REST API".to_owned(),
            direction: "source".to_owned(),
            host: "api.example.internal".to_owned(),
            secret_ref: "env:INGEST_SPEC_DEFAULTS_TEST_TOKEN".to_owned(),
            secret_ref_secondary: None,
            environment: "staging".to_owned(),
            tenant: "Meridian Group".to_owned(),
            residency: "in-region".to_owned(),
            capabilities: vec![],
            owner: None,
        },
    )
    .await
    .unwrap();

    let spec = get_ingest_spec(&pool, &created.id).await.unwrap().unwrap();
    assert_eq!(spec.adapter, None);
    assert_eq!(spec.ingest_mode, None);
    assert_eq!(spec.dial, serde_json::json!({}));
    assert_eq!(spec.source_objects, serde_json::json!([]));
    assert_eq!(spec.schedule_cron, None);
    Ok(())
}

/// `0034_seed_connector_ingest_spec.sql` fills in `adapter`/`dial`/
/// `source_objects` on the two connectors this compose stack can actually
/// dial (`conn-pg-lakehouse`, `conn-s3-warehouse`); this pins that BOTH
/// rows moved off the column defaults, complementing
/// `tests/connector_ingest_spec_seed.rs`'s per-field assertions on each
/// row.
#[sqlx::test(migrations = "../../migrations")]
async fn seeded_connectors_no_longer_have_null_adapter(pool: PgPool) -> sqlx::Result<()> {
    let rows: Vec<(Option<String>,)> = sqlx::query_as(
        "SELECT adapter FROM connector WHERE id IN ('conn-pg-lakehouse', 'conn-s3-warehouse') \
         ORDER BY id",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(rows.len(), 2, "expected both seeded connectors to exist");
    for (adapter,) in rows {
        assert!(adapter.is_some(), "expected 0034 to have set adapter");
    }
    Ok(())
}

/// `connector_adapter_check` rejects any value outside the five documented
/// adapters.
#[sqlx::test(migrations = "../../migrations")]
async fn adapter_check_constraint_rejects_an_unknown_adapter(pool: PgPool) -> sqlx::Result<()> {
    let result = sqlx::query(
        "UPDATE connector SET adapter = 'ftp' WHERE id = \
         (SELECT id FROM connector LIMIT 1)",
    )
    .execute(&pool)
    .await;
    assert!(
        result.is_err(),
        "expected the CHECK constraint to reject 'ftp'"
    );
    Ok(())
}

/// `connector_ingest_mode_check` rejects any value outside `batch`/`cdc`.
#[sqlx::test(migrations = "../../migrations")]
async fn ingest_mode_check_constraint_rejects_an_unknown_mode(pool: PgPool) -> sqlx::Result<()> {
    let result = sqlx::query(
        "UPDATE connector SET ingest_mode = 'streaming' WHERE id = \
         (SELECT id FROM connector LIMIT 1)",
    )
    .execute(&pool)
    .await;
    assert!(
        result.is_err(),
        "expected the CHECK constraint to reject 'streaming'"
    );
    Ok(())
}
