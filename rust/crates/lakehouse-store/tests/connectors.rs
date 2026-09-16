//! Integration tests for `lakehouse_store::connectors` against a real
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

use lakehouse_store::StoreError;
use lakehouse_store::audit::{NewAuditEvent, insert as insert_audit_event};
use lakehouse_store::connectors::{
    CreateConnectorInput, IngestSpecInput, create_connector, delete_connector, get_connector,
    get_connector_dial_info, get_ingest_spec, list_connectors, record_test_result, set_ingest_spec,
};
use lakehouse_store::pipelines::{CreatePipelineInput, create_pipeline};
use sqlx::PgPool;

/// P6 shrank the seed to the two connector types this build can actually
/// dial (`0022_prune_connector_seed.sql`) — see that migration's header
/// comment for why the 28-row `mock/connectors.ts` fixture was removed.
#[sqlx::test(migrations = "../../migrations")]
async fn seed_populates_connector_list(pool: PgPool) -> sqlx::Result<()> {
    let connectors = list_connectors(&pool).await.unwrap();
    assert_eq!(connectors.len(), 2);
    assert!(connectors.iter().any(|c| c.id == "conn-pg-lakehouse"));
    assert!(connectors.iter().any(|c| c.id == "conn-s3-warehouse"));
    Ok(())
}

/// The whole point of this domain: no read path can round-trip a `host` or
/// `secretRef` back to a caller, no matter how the row was created.
#[sqlx::test(migrations = "../../migrations")]
async fn created_connector_never_carries_host_or_secret_ref_on_the_wire(
    pool: PgPool,
) -> sqlx::Result<()> {
    let input = CreateConnectorInput {
        name: "leak test connector".to_owned(),
        kind: "REST API".to_owned(),
        direction: "source".to_owned(),
        host: "super-secret-internal-host.example".to_owned(),
        secret_ref: "env:LEAK_TEST_TOKEN".to_owned(),
        secret_ref_secondary: None,
        environment: "staging".to_owned(),
        tenant: "Meridian Group".to_owned(),
        residency: "in-region".to_owned(),
        capabilities: vec![],
        owner: None,
    };
    let created = create_connector(&pool, &input).await.unwrap();

    let as_json = serde_json::to_value(&created).unwrap();
    assert!(as_json.get("host").is_none());
    assert!(as_json.get("secretRef").is_none());
    let raw = serde_json::to_string(&as_json).unwrap();
    assert!(!raw.contains("super-secret-internal-host"));
    assert!(!raw.contains("LEAK_TEST_TOKEN"));

    // Also true of the list/detail reads, not just the create response.
    let list = list_connectors(&pool).await.unwrap();
    let list_json = serde_json::to_string(&list).unwrap();
    assert!(!list_json.contains("super-secret-internal-host"));
    assert!(!list_json.contains("LEAK_TEST_TOKEN"));

    let detail = get_connector(&pool, &created.id).await.unwrap().unwrap();
    let detail_json = serde_json::to_string(&detail).unwrap();
    assert!(!detail_json.contains("super-secret-internal-host"));
    assert!(!detail_json.contains("LEAK_TEST_TOKEN"));

    Ok(())
}

/// A duplicate name is a 409, matching `pipeline_definition_name_unique`'s
/// treatment.
#[sqlx::test(migrations = "../../migrations")]
async fn create_connector_rejects_duplicate_name(pool: PgPool) -> sqlx::Result<()> {
    let input = CreateConnectorInput {
        name: "Lakehouse OLTP (Postgres)".to_owned(), // seeded name
        kind: "REST API".to_owned(),
        direction: "source".to_owned(),
        host: "h".to_owned(),
        secret_ref: "env:X".to_owned(),
        secret_ref_secondary: None,
        environment: "staging".to_owned(),
        tenant: "Meridian Group".to_owned(),
        residency: String::new(),
        capabilities: vec![],
        owner: None,
    };
    let err = create_connector(&pool, &input).await.unwrap_err();
    assert!(matches!(err, StoreError::Conflict));
    Ok(())
}

/// `get_connector` derives `dependentPipelines` from
/// `pipeline_definition.connector_id`, live — not from a stored/denormalized
/// column.
#[sqlx::test(migrations = "../../migrations")]
async fn dependent_pipelines_are_derived_from_pipeline_definition(
    pool: PgPool,
) -> sqlx::Result<()> {
    let connector = create_connector(
        &pool,
        &CreateConnectorInput {
            name: "dependents test connector".to_owned(),
            kind: "REST API".to_owned(),
            direction: "source".to_owned(),
            host: "h".to_owned(),
            secret_ref: "env:X".to_owned(),
            secret_ref_secondary: None,
            environment: "staging".to_owned(),
            tenant: "Meridian Group".to_owned(),
            residency: String::new(),
            capabilities: vec![],
            owner: None,
        },
    )
    .await
    .unwrap();

    let before = get_connector(&pool, &connector.id).await.unwrap().unwrap();
    assert!(before.dependent_pipelines.is_empty());

    let pipeline = create_pipeline(
        &pool,
        &CreatePipelineInput {
            name: "dependents test pipeline".to_owned(),
            kind: "batch".to_owned(),
            source_zone: "bronze".to_owned(),
            source_table: "t".to_owned(),
            target_zone: "silver".to_owned(),
            target_table: "t".to_owned(),
            schedule: "manual".to_owned(),
            owner: None,
        },
    )
    .await
    .unwrap();
    sqlx::query("UPDATE pipeline_definition SET connector_id = $1 WHERE id = $2")
        .bind(&connector.id)
        .bind(&pipeline.id)
        .execute(&pool)
        .await
        .unwrap();

    let after = get_connector(&pool, &connector.id).await.unwrap().unwrap();
    assert_eq!(after.dependent_pipelines.len(), 1);
    assert_eq!(after.dependent_pipelines[0].id, pipeline.id);
    assert_eq!(after.dependent_pipelines[0].kind, "pipeline");
    Ok(())
}

/// `get_connector_dial_info` hands back exactly the fields a real probe
/// needs — including `secret_ref_secondary` for the S3 connector, which
/// `get_connector`/`list_connectors` never expose at all.
#[sqlx::test(migrations = "../../migrations")]
async fn dial_info_returns_type_host_and_secret_refs(pool: PgPool) -> sqlx::Result<()> {
    let pg = get_connector_dial_info(&pool, "conn-pg-lakehouse")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pg.kind, "PostgreSQL");
    assert_eq!(pg.host, "lakehouse@postgres:5432/lakehouse");
    // Connector-DEDICATED refs, not the API's own secrets — migration 0023.
    // The seed used to name `env:POSTGRES_PASSWORD` (the console's own
    // database password) and the RustFS root keys, which made them reachable
    // by name from a caller-created connector pointed at a host of the
    // caller's choosing. Asserting the new names here keeps that regression
    // visible: re-seeding the API's own secrets fails this test.
    assert_eq!(pg.secret_ref, "env:CONNECTOR_PG_PASSWORD");
    assert_eq!(pg.secret_ref_secondary, None);

    let s3 = get_connector_dial_info(&pool, "conn-s3-warehouse")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(s3.kind, "Object storage");
    assert_eq!(s3.secret_ref, "env:CONNECTOR_S3_ACCESS_KEY");
    assert_eq!(
        s3.secret_ref_secondary.as_deref(),
        Some("env:CONNECTOR_S3_SECRET_KEY")
    );

    assert!(
        get_connector_dial_info(&pool, "conn-does-not-exist")
            .await
            .unwrap()
            .is_none()
    );
    Ok(())
}

/// `record_test_result` persists exactly what the caller measured — never
/// derives `ok`/`latency_ms` itself — and stamps `lastTestAt` forward.
/// `health` follows `ok` only when `supported` is `true`.
#[sqlx::test(migrations = "../../migrations")]
async fn record_test_result_persists_outcome_and_stamps_last_test_at(
    pool: PgPool,
) -> sqlx::Result<()> {
    let before = get_connector(&pool, "conn-pg-lakehouse")
        .await
        .unwrap()
        .unwrap();

    let ok_result = record_test_result(&pool, "conn-pg-lakehouse", true, true, Some(12), "ok")
        .await
        .unwrap();
    assert!(ok_result.ok);
    assert!(ok_result.supported);
    assert_eq!(ok_result.latency_ms, Some(12));
    let after_ok = get_connector(&pool, "conn-pg-lakehouse")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after_ok.connector.health, "healthy");
    assert_ne!(
        before.connector.last_test_at, after_ok.connector.last_test_at,
        "record_test_result must stamp lastTestAt forward"
    );

    let fail_result = record_test_result(
        &pool,
        "conn-pg-lakehouse",
        false,
        true,
        Some(4999),
        "refused",
    )
    .await
    .unwrap();
    assert!(!fail_result.ok);
    let after_fail = get_connector(&pool, "conn-pg-lakehouse")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after_fail.connector.health, "unhealthy");

    // Unsupported: health is left untouched, and no latency is recorded.
    let before_unsupported = get_connector(&pool, "conn-pg-lakehouse")
        .await
        .unwrap()
        .unwrap();
    let unsupported_result = record_test_result(
        &pool,
        "conn-pg-lakehouse",
        false,
        false,
        None,
        "unsupported",
    )
    .await
    .unwrap();
    assert!(!unsupported_result.ok);
    assert!(!unsupported_result.supported);
    assert_eq!(unsupported_result.latency_ms, None);
    let after_unsupported = get_connector(&pool, "conn-pg-lakehouse")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        after_unsupported.connector.health, before_unsupported.connector.health,
        "an unsupported test must not change stored health"
    );
    Ok(())
}

/// Testing a connector that doesn't exist is a `NotFound`, not a panic or a
/// silently-empty success.
#[sqlx::test(migrations = "../../migrations")]
async fn record_test_result_not_found_for_unknown_id(pool: PgPool) -> sqlx::Result<()> {
    let err = record_test_result(&pool, "conn-does-not-exist", true, true, Some(1), "x")
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::NotFound));
    Ok(())
}

/// Fetching an unknown connector's detail returns `None`, not an error.
#[sqlx::test(migrations = "../../migrations")]
async fn get_connector_none_for_unknown_id(pool: PgPool) -> sqlx::Result<()> {
    let detail = get_connector(&pool, "conn-does-not-exist").await.unwrap();
    assert!(detail.is_none());
    Ok(())
}

/// `delete_connector` actually removes the row, and reports `true`.
#[sqlx::test(migrations = "../../migrations")]
async fn delete_connector_removes_the_row(pool: PgPool) -> sqlx::Result<()> {
    let input = CreateConnectorInput {
        name: "delete me".to_owned(),
        kind: "REST API".to_owned(),
        direction: "source".to_owned(),
        host: "h".to_owned(),
        secret_ref: "env:DELETE_ME".to_owned(),
        secret_ref_secondary: None,
        environment: "staging".to_owned(),
        tenant: "Meridian Group".to_owned(),
        residency: String::new(),
        capabilities: vec![],
        owner: None,
    };
    let created = create_connector(&pool, &input).await.unwrap();

    let deleted = delete_connector(&pool, &created.id).await.unwrap();
    assert!(deleted);
    assert!(get_connector(&pool, &created.id).await.unwrap().is_none());
    Ok(())
}

/// Deleting an unknown id is `Ok(false)`, not an error — idempotent-delete
/// convention.
#[sqlx::test(migrations = "../../migrations")]
async fn delete_connector_unknown_id_is_false_not_an_error(pool: PgPool) -> sqlx::Result<()> {
    let deleted = delete_connector(&pool, "conn-does-not-exist")
        .await
        .unwrap();
    assert!(!deleted);
    Ok(())
}

/// WS1 finding J19: a brand-new connector must never claim, at creation, a
/// health status or a test time it does not have. `create_connector` starts
/// `health = "unknown"` and both timestamps `None` -- `record_test_result`
/// is the only thing that ever moves them.
#[sqlx::test(migrations = "../../migrations")]
async fn a_new_connector_starts_unknown_and_untested(pool: PgPool) -> sqlx::Result<()> {
    let created = create_connector(
        &pool,
        &CreateConnectorInput {
            name: "freshly created connector".to_owned(),
            kind: "REST API".to_owned(),
            direction: "source".to_owned(),
            host: "h".to_owned(),
            secret_ref: "env:X".to_owned(),
            secret_ref_secondary: None,
            environment: "staging".to_owned(),
            tenant: "Meridian Group".to_owned(),
            residency: String::new(),
            capabilities: vec![],
            owner: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(created.health, "unknown");
    assert!(created.last_test_at.is_none());
    assert!(created.last_activity_at.is_none());

    // Also true of a fresh read, not just the create response.
    let read_back = get_connector(&pool, &created.id).await.unwrap().unwrap();
    assert_eq!(read_back.connector.health, "unknown");
    assert!(read_back.connector.last_test_at.is_none());
    assert!(read_back.connector.last_activity_at.is_none());
    Ok(())
}

/// An unsupported probe type never actually dialed the connector, so it
/// must not stamp a test time it does not have -- only `health` for a
/// SUPPORTED probe is meaningful evidence.
#[sqlx::test(migrations = "../../migrations")]
async fn an_unsupported_probe_does_not_stamp_a_test_time(pool: PgPool) -> sqlx::Result<()> {
    let created = create_connector(
        &pool,
        &CreateConnectorInput {
            name: "unsupported probe connector".to_owned(),
            kind: "Kafka".to_owned(),
            direction: "source".to_owned(),
            host: "h".to_owned(),
            secret_ref: "env:X".to_owned(),
            secret_ref_secondary: None,
            environment: "staging".to_owned(),
            tenant: "Meridian Group".to_owned(),
            residency: String::new(),
            capabilities: vec![],
            owner: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(created.health, "unknown");
    assert!(created.last_test_at.is_none());

    let result = record_test_result(&pool, &created.id, false, false, None, "unsupported")
        .await
        .unwrap();
    assert!(!result.supported);
    assert!(result.tested_at.is_none());

    let after = get_connector(&pool, &created.id).await.unwrap().unwrap();
    assert_eq!(
        after.connector.health, "unknown",
        "an unsupported probe must not change stored health"
    );
    assert!(
        after.connector.last_test_at.is_none(),
        "an unsupported probe never actually dialed the connector, so it must not claim a test \
         time"
    );
    Ok(())
}

/// A SUPPORTED probe that fails is real evidence: it stamps both a test
/// time and `health`.
#[sqlx::test(migrations = "../../migrations")]
async fn a_supported_probe_stamps_time_and_health(pool: PgPool) -> sqlx::Result<()> {
    let created = create_connector(
        &pool,
        &CreateConnectorInput {
            name: "supported probe connector".to_owned(),
            kind: "REST API".to_owned(),
            direction: "source".to_owned(),
            host: "h".to_owned(),
            secret_ref: "env:X".to_owned(),
            secret_ref_secondary: None,
            environment: "staging".to_owned(),
            tenant: "Meridian Group".to_owned(),
            residency: String::new(),
            capabilities: vec![],
            owner: None,
        },
    )
    .await
    .unwrap();

    let result = record_test_result(&pool, &created.id, false, true, Some(42), "refused")
        .await
        .unwrap();
    assert!(result.supported);
    assert!(!result.ok);
    assert!(result.tested_at.is_some());

    let after = get_connector(&pool, &created.id).await.unwrap().unwrap();
    assert_eq!(after.connector.health, "unhealthy");
    assert!(after.connector.last_test_at.is_some());
    Ok(())
}

/// `0028_connector_health_unknown_until_tested.sql` resets the two seeded
/// connectors' fabricated `health`/`last_test_at` -- after full migration
/// (which every `#[sqlx::test]` here runs), both read as `"unknown"`/`None`.
#[sqlx::test(migrations = "../../migrations")]
async fn seeded_connectors_read_unknown_and_untested_after_migration(
    pool: PgPool,
) -> sqlx::Result<()> {
    for id in ["conn-pg-lakehouse", "conn-s3-warehouse"] {
        let detail = get_connector(&pool, id).await.unwrap().unwrap();
        assert_eq!(
            detail.connector.health, "unknown",
            "{id} should read unknown"
        );
        assert!(
            detail.connector.last_test_at.is_none(),
            "{id} should read untested"
        );
    }
    Ok(())
}

/// `sqlx::test` always applies every migration, so `0028`'s targeted
/// predicate (`last_test_at < created_at`) cannot be exercised by running
/// migrations partially. Instead this test proves the predicate is
/// targeted by executing the migration's own UPDATE statement text a
/// SECOND time, directly, against a row crafted to represent a genuine
/// post-creation test result (`last_test_at` AFTER `created_at`) — the
/// shape `record_test_result` always produces, and the opposite of the
/// fabricated seed shape the migration is meant to catch. If the predicate
/// were a blanket `UPDATE ... WHERE id IN (...)` (not also gated on
/// `last_test_at < created_at`), this row would incorrectly get reset too.
#[sqlx::test(migrations = "../../migrations")]
async fn migration_predicate_never_resets_a_genuine_post_creation_test(
    pool: PgPool,
) -> sqlx::Result<()> {
    // Replace the seeded row with one shaped like a real, already-tested
    // connector: `last_test_at` AFTER `created_at`.
    sqlx::query("DELETE FROM connector WHERE id = $1")
        .bind("conn-pg-lakehouse")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO connector (id, name, type, direction, health, environment, tenant, host, \
         secret_ref, created_at, last_test_at) VALUES ($1, $2, $3, $4, 'healthy', $5, $6, $7, \
         $8, now() - interval '1 hour', now())",
    )
    .bind("conn-pg-lakehouse")
    .bind("genuinely tested connector")
    .bind("PostgreSQL")
    .bind("source")
    .bind("production")
    .bind("Meridian Group")
    .bind("h")
    .bind("env:X")
    .execute(&pool)
    .await
    .unwrap();

    // Re-run 0028's own UPDATE statement text (not the whole migration
    // file, which already applied once during provisioning).
    sqlx::query(
        "UPDATE connector SET health = 'unknown', last_test_at = NULL WHERE id IN \
         ('conn-pg-lakehouse', 'conn-s3-warehouse') AND health = 'healthy' AND last_test_at < \
         created_at",
    )
    .execute(&pool)
    .await
    .unwrap();

    let after = get_connector(&pool, "conn-pg-lakehouse")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        after.connector.health, "healthy",
        "a genuine post-creation test result must survive the migration's predicate"
    );
    assert!(
        after.connector.last_test_at.is_some(),
        "a genuine post-creation test result must survive the migration's predicate"
    );
    Ok(())
}

/// WS1 task 1.14 (judge finding J12): `get_connector` used to compute
/// `aud-conn-<id>` on every read — a string that named no `audit_event`
/// row, so "View audit" always 404ed. It now resolves the id only when a
/// real event exists for `resource_kind = 'connector'` /
/// `resource_id = <connector id>`.
#[sqlx::test(migrations = "../../migrations")]
async fn get_connector_audit_event_id_resolves_a_real_event(pool: PgPool) -> sqlx::Result<()> {
    let before = get_connector(&pool, "conn-pg-lakehouse")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(before.audit_event_id, None);

    let event = insert_audit_event(
        &pool,
        NewAuditEvent {
            action: "test_connection".to_owned(),
            resource_kind: Some("connector".to_owned()),
            resource_id: Some("conn-pg-lakehouse".to_owned()),
            outcome: "executed".to_owned(),
            ..NewAuditEvent::default()
        },
    )
    .await
    .unwrap();

    let after = get_connector(&pool, "conn-pg-lakehouse")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.audit_event_id.as_deref(), Some(event.id.as_str()));
    Ok(())
}

/// A minimal, valid [`CreateConnectorInput`] for tests that only care about
/// the resulting connector's id, not its other fields.
fn minimal_input(name: &str) -> CreateConnectorInput {
    CreateConnectorInput {
        name: name.to_owned(),
        kind: "REST API".to_owned(),
        direction: "source".to_owned(),
        host: "api.example.internal".to_owned(),
        secret_ref: "env:INGEST_SPEC_TEST_TOKEN".to_owned(),
        secret_ref_secondary: None,
        environment: "staging".to_owned(),
        tenant: "Meridian Group".to_owned(),
        residency: "in-region".to_owned(),
        capabilities: vec![],
        owner: None,
    }
}

/// `set_ingest_spec` writes a valid `dial`, and `get_ingest_spec` reads it
/// back, including the connector's existing `secretRef` (never resolved,
/// only named — see `IngestSecretRefs`'s doc comment).
#[sqlx::test(migrations = "../../migrations")]
async fn set_ingest_spec_then_get_round_trips(pool: PgPool) -> sqlx::Result<()> {
    let created = create_connector(&pool, &minimal_input("ingest spec round trip"))
        .await
        .unwrap();
    let spec = IngestSpecInput {
        adapter: "files".to_owned(),
        ingest_mode: "batch".to_owned(),
        dial: serde_json::json!({"protocol": "s3", "bucket": "b", "format": "csv"}),
        source_objects: serde_json::json!([]),
        schedule_cron: Some("0 * * * *".to_owned()),
    };
    set_ingest_spec(&pool, &created.id, &spec).await.unwrap();

    let read = get_ingest_spec(&pool, &created.id).await.unwrap().unwrap();
    assert_eq!(read.adapter.as_deref(), Some("files"));
    assert_eq!(read.ingest_mode.as_deref(), Some("batch"));
    assert_eq!(read.dial, spec.dial);
    assert_eq!(read.schedule_cron.as_deref(), Some("0 * * * *"));
    assert_eq!(read.secret_refs.primary, "env:INGEST_SPEC_TEST_TOKEN");
    assert_eq!(read.secret_refs.secondary, None);
    Ok(())
}

/// A `dial` that fails its adapter's own validation (`sql` requires
/// `driver`/`host`/`port`/`database`/`user`, and rejects an unknown field
/// like `password`) is rejected as [`StoreError::Validation`] and never
/// written — the whole point of validating BEFORE the `UPDATE`.
#[sqlx::test(migrations = "../../migrations")]
async fn set_ingest_spec_rejects_a_dial_that_fails_adapter_validation(
    pool: PgPool,
) -> sqlx::Result<()> {
    let created = create_connector(&pool, &minimal_input("ingest spec invalid dial"))
        .await
        .unwrap();
    let spec = IngestSpecInput {
        adapter: "sql".to_owned(),
        ingest_mode: "batch".to_owned(),
        // Missing required fields (host/port/database/user) AND carries a
        // forbidden `password` field -- `SqlDial`'s `deny_unknown_fields`
        // rejects it before any missing-field check even runs.
        dial: serde_json::json!({"driver": "mysql", "password": "s3cret"}),
        source_objects: serde_json::json!([]),
        schedule_cron: None,
    };
    let err = set_ingest_spec(&pool, &created.id, &spec)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::Validation(_)));

    // The rejected dial must never have reached the row.
    let read = get_ingest_spec(&pool, &created.id).await.unwrap().unwrap();
    assert_eq!(
        read.adapter, None,
        "an invalid dial must never be persisted"
    );
    assert_eq!(read.dial, serde_json::json!({}));
    Ok(())
}

/// `set_ingest_spec`/`get_ingest_spec` both return [`StoreError::NotFound`]
/// / `Ok(None)` for an id that names no connector, rather than silently
/// succeeding or panicking.
#[sqlx::test(migrations = "../../migrations")]
async fn ingest_spec_functions_treat_an_unknown_id_honestly(pool: PgPool) -> sqlx::Result<()> {
    assert!(
        get_ingest_spec(&pool, "conn-does-not-exist")
            .await
            .unwrap()
            .is_none()
    );

    let spec = IngestSpecInput {
        adapter: "files".to_owned(),
        ingest_mode: "batch".to_owned(),
        dial: serde_json::json!({"protocol": "s3", "bucket": "b", "format": "csv"}),
        source_objects: serde_json::json!([]),
        schedule_cron: None,
    };
    let err = set_ingest_spec(&pool, "conn-does-not-exist", &spec)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::NotFound));
    Ok(())
}
