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
/// defaults, so an existing connector row keeps parsing with
/// `adapter = NULL` ("not ingestible yet") rather than failing to migrate.
///
/// This test originally asserted `adapter = NULL` for every seeded
/// connector row, but `0034_seed_connector_ingest_spec.sql` now fills in
/// `adapter`/`dial`/`source_objects` on the two rows
/// `0022_prune_connector_seed.sql` seeded (`conn-pg-lakehouse`,
/// `conn-s3-warehouse`) — the two connectors this compose stack can
/// actually dial. Those two are excluded here and covered instead by
/// `tests/connector_ingest_spec_seed.rs`; this test now checks the
/// column-default claim against every OTHER connector row, so it stays
/// meaningful if a future migration seeds a third dialable connector
/// without touching this one.
#[sqlx::test(migrations = "../../migrations")]
async fn non_seeded_connectors_have_null_adapter_and_empty_dial(pool: PgPool) -> sqlx::Result<()> {
    type ConnectorIngestRow = (
        Option<String>,
        Option<String>,
        serde_json::Value,
        serde_json::Value,
        Option<String>,
    );
    let rows: Vec<ConnectorIngestRow> = sqlx::query_as(
        "SELECT adapter, ingest_mode, dial, source_objects, schedule_cron FROM connector \
             WHERE id NOT IN ('conn-pg-lakehouse', 'conn-s3-warehouse') ORDER BY id",
    )
    .fetch_all(&pool)
    .await?;
    for (adapter, ingest_mode, dial, source_objects, schedule_cron) in rows {
        assert_eq!(adapter, None);
        assert_eq!(ingest_mode, None);
        assert_eq!(dial, serde_json::json!({}));
        assert_eq!(source_objects, serde_json::json!([]));
        assert_eq!(schedule_cron, None);
    }
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
