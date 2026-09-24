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
use lakehouse_store::connector_probe_result::list_probe_results;
use lakehouse_store::connectors::{
    ConnectorFilter, CreateConnectorInput, CredentialKind, CredentialSource, CredentialSpec,
    IngestSpecInput, SecretSlot, create_connector, delete_connector, get_connector,
    get_connector_dial_info, get_ingest_spec, list_connectors, list_ingestible_connectors,
    record_test_result, set_ingest_spec, swap_secret_ref,
};
use lakehouse_store::identity::{CreateTenantInput, create_tenant};
use lakehouse_store::pipelines::{CreatePipelineInput, create_pipeline};
use sqlx::PgPool;
use uuid::Uuid;

/// A single-slot `env:`-sourced credential spec -- the shape every test in
/// this file that does not care about the specific derived name uses.
fn single_credential() -> CredentialSpec {
    CredentialSpec {
        source: CredentialSource::Env,
        primary: CredentialKind::Token,
        secondary: None,
    }
}

/// A two-slot `env:`-sourced credential spec -- `primary`/`secondary` use
/// DIFFERENT kinds (`Token`/`SecretKey`), not the same one twice: the same
/// id + source + kind always derives the SAME name
/// ([`derive_secret_ref`]'s whole contract), so two slots that need to be
/// distinct names must pick distinct kinds.
fn two_slot_credential() -> CredentialSpec {
    CredentialSpec {
        source: CredentialSource::Env,
        primary: CredentialKind::Token,
        secondary: Some(CredentialKind::SecretKey),
    }
}

/// P6 shrank the seed to the two connector types this build can actually
/// dial (`0022_prune_connector_seed.sql`) — see that migration's header
/// comment for why the 28-row `mock/connectors.ts` fixture was removed.
#[sqlx::test(migrations = "../../migrations")]
async fn seed_populates_connector_list(pool: PgPool) -> sqlx::Result<()> {
    let connectors = list_connectors(&pool, &ConnectorFilter::default())
        .await
        .unwrap();
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
        credential: single_credential(),
        environment: "staging".to_owned(),
        tenant: "Meridian Group".to_owned(),
        residency: "in-region".to_owned(),
        capabilities: vec![],
        owner: None,
    };
    let (created, credential_names) = create_connector(&pool, &input).await.unwrap();

    let as_json = serde_json::to_value(&created).unwrap();
    assert!(as_json.get("host").is_none());
    assert!(as_json.get("secretRef").is_none());
    let raw = serde_json::to_string(&as_json).unwrap();
    assert!(!raw.contains("super-secret-internal-host"));
    assert!(!raw.contains(&credential_names.primary));

    // Also true of the list/detail reads, not just the create response.
    let list = list_connectors(&pool, &ConnectorFilter::default())
        .await
        .unwrap();
    let list_json = serde_json::to_string(&list).unwrap();
    assert!(!list_json.contains("super-secret-internal-host"));
    assert!(!list_json.contains(&credential_names.primary));

    let detail = get_connector(&pool, &created.id).await.unwrap().unwrap();
    let detail_json = serde_json::to_string(&detail).unwrap();
    assert!(!detail_json.contains("super-secret-internal-host"));
    assert!(!detail_json.contains(&credential_names.primary));

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
        credential: single_credential(),
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
    let (connector, _credential_names) = create_connector(
        &pool,
        &CreateConnectorInput {
            name: "dependents test connector".to_owned(),
            kind: "REST API".to_owned(),
            direction: "source".to_owned(),
            host: "h".to_owned(),
            credential: single_credential(),
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
            incremental_column: None,
            transforms: Vec::new(),
            fbic_enabled: false,
            target_zone: "silver".to_owned(),
            target_table: "t".to_owned(),
            schedule: "manual".to_owned(),
            owner: None,
            description: None,
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
        credential: single_credential(),
        environment: "staging".to_owned(),
        tenant: "Meridian Group".to_owned(),
        residency: String::new(),
        capabilities: vec![],
        owner: None,
    };
    let (created, _credential_names) = create_connector(&pool, &input).await.unwrap();

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
    let (created, _credential_names) = create_connector(
        &pool,
        &CreateConnectorInput {
            name: "freshly created connector".to_owned(),
            kind: "REST API".to_owned(),
            direction: "source".to_owned(),
            host: "h".to_owned(),
            credential: single_credential(),
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
    let (created, _credential_names) = create_connector(
        &pool,
        &CreateConnectorInput {
            name: "unsupported probe connector".to_owned(),
            kind: "Kafka".to_owned(),
            direction: "source".to_owned(),
            host: "h".to_owned(),
            credential: single_credential(),
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
    let (created, _credential_names) = create_connector(
        &pool,
        &CreateConnectorInput {
            name: "supported probe connector".to_owned(),
            kind: "REST API".to_owned(),
            direction: "source".to_owned(),
            host: "h".to_owned(),
            credential: single_credential(),
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
        credential: single_credential(),
        environment: "staging".to_owned(),
        tenant: "Meridian Group".to_owned(),
        residency: "in-region".to_owned(),
        capabilities: vec![],
        owner: None,
    }
}

/// A valid `cdc`-adapter `dial`, matching `ingest_spec::CdcDial`'s required
/// fields (`driver`/`host`/`port`/`database`/`user`/`slotName`/
/// `publicationName`).
fn cdc_spec_fixture() -> IngestSpecInput {
    IngestSpecInput {
        adapter: "cdc".to_owned(),
        ingest_mode: "cdc".to_owned(),
        dial: serde_json::json!({
            "driver": "postgres",
            "host": "source.example.internal",
            "port": 5432,
            "database": "oms",
            "user": "replicator",
            "slotName": "oms_orders_slot",
            "publicationName": "oms_orders_pub",
        }),
        source_objects: serde_json::json!([]),
        schedule_cron: None,
    }
}

/// [`ConnectorDialInfo`] must hand back `adapter`/`dial` alongside the
/// connectivity fields it already returned — `routes::connectors`'s
/// deprovision-on-delete dispatch (WS3 plan review X4) needs both to decide
/// whether a connector's replication slot/publication should be attempted
/// at all, and, for a `cdc` adapter, to read the names straight out of
/// `dial` rather than guessing them from the connector's `id`.
#[sqlx::test(migrations = "../../migrations")]
async fn get_connector_dial_info_includes_adapter_and_dial(pool: PgPool) -> sqlx::Result<()> {
    let (created, _credential_names) =
        create_connector(&pool, &minimal_input("dial info adapter and dial"))
            .await
            .unwrap();
    let spec = cdc_spec_fixture();
    set_ingest_spec(&pool, &created.id, &spec).await.unwrap();

    let info = get_connector_dial_info(&pool, &created.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(info.adapter.as_deref(), Some("cdc"));
    assert_eq!(info.dial, spec.dial);
    Ok(())
}

/// `set_ingest_spec` writes a valid `dial`, and `get_ingest_spec` reads it
/// back, including the connector's existing `secretRef` (never resolved,
/// only named — see `IngestSecretRefs`'s doc comment).
///
/// Uses `sql`, not `files`: `secret_field_names("files", None)` needs
/// TWO secret refs (`accessKey`+`secretKey`, WS3 plan review Z6), so a
/// `files` spec would no longer round-trip against `minimal_input`'s
/// single `secretRef` — this test is about the round trip itself, not
/// that particular adapter's secret-count rule, so it picks the adapter
/// (`sql`) whose single-field mapping matches `minimal_input` as-is,
/// letting this test still assert `secret_refs.secondary == None`
/// meaningfully.
#[sqlx::test(migrations = "../../migrations")]
async fn set_ingest_spec_then_get_round_trips(pool: PgPool) -> sqlx::Result<()> {
    let (created, credential_names) =
        create_connector(&pool, &minimal_input("ingest spec round trip"))
            .await
            .unwrap();
    let spec = IngestSpecInput {
        adapter: "sql".to_owned(),
        ingest_mode: "batch".to_owned(),
        dial: serde_json::json!({
            "driver": "postgres",
            "host": "source.example.internal",
            "port": 5432,
            "database": "orders",
            "user": "app_reader",
        }),
        source_objects: serde_json::json!([]),
        schedule_cron: Some("0 * * * *".to_owned()),
    };
    set_ingest_spec(&pool, &created.id, &spec).await.unwrap();

    let read = get_ingest_spec(&pool, &created.id).await.unwrap().unwrap();
    assert_eq!(read.adapter.as_deref(), Some("sql"));
    assert_eq!(read.ingest_mode.as_deref(), Some("batch"));
    assert_eq!(read.dial, spec.dial);
    assert_eq!(read.schedule_cron.as_deref(), Some("0 * * * *"));
    assert_eq!(read.secret_refs.primary, credential_names.primary);
    assert_eq!(read.secret_refs.secondary, None);
    Ok(())
}

/// `list_ingestible_connectors` carries the connector's
/// `secretRef` NAME (never resolved), and only rows that have had an
/// ingest spec set at all (`adapter IS NOT NULL`) — a connector created
/// but never given an ingest spec must not show up as "ingestible".
#[sqlx::test(migrations = "../../migrations")]
async fn list_ingestible_connectors_carries_the_secret_ref_name_never_resolved(
    pool: PgPool,
) -> sqlx::Result<()> {
    let (never_configured, _credential_names) =
        create_connector(&pool, &minimal_input("never given an ingest spec"))
            .await
            .unwrap();

    let (created, credential_names) = create_connector(&pool, &minimal_input("ingestible listing"))
        .await
        .unwrap();
    let spec = IngestSpecInput {
        adapter: "sql".to_owned(),
        ingest_mode: "batch".to_owned(),
        dial: serde_json::json!({
            "driver": "mysql",
            "host": "source.example.internal",
            "port": 3306,
            "database": "orders",
            "user": "app_reader",
        }),
        source_objects: serde_json::json!([{"name": "orders", "target": "orders"}]),
        schedule_cron: Some("0 * * * *".to_owned()),
    };
    set_ingest_spec(&pool, &created.id, &spec).await.unwrap();

    let rows = list_ingestible_connectors(&pool).await.unwrap();
    assert!(
        !rows.iter().any(|r| r.id == never_configured.id),
        "a connector with no ingest spec set must not be listed as ingestible"
    );

    let row = rows.iter().find(|r| r.id == created.id).unwrap();
    assert_eq!(row.adapter, "sql");
    assert_eq!(row.ingest_mode, "batch");
    assert_eq!(row.dial, spec.dial);
    assert_eq!(row.source_objects, spec.source_objects);
    assert_eq!(row.schedule_cron.as_deref(), Some("0 * * * *"));
    assert_eq!(row.secret_ref, credential_names.primary);
    assert_eq!(row.secret_ref_secondary, None);
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
    let (created, _credential_names) =
        create_connector(&pool, &minimal_input("ingest spec invalid dial"))
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

/// WS3 plan review Z6: a `rest` connector saved with `auth.type = "basic"`
/// needs TWO secret refs (`secret_map.secret_field_names`/
/// `ingest_spec::secret_field_names` both map `("rest", "basic")` to
/// `("username", "password")`), but `minimal_input` only ever sets
/// `secret_ref`, leaving `secret_ref_secondary` `None`. `set_ingest_spec`
/// must reject this at SAVE time, not let it through to be discovered as
/// a `KeyError` inside `secret_resolver.resolve_secrets` at run time.
#[sqlx::test(migrations = "../../migrations")]
async fn set_ingest_spec_rejects_basic_auth_rest_with_only_one_secret_ref(
    pool: PgPool,
) -> sqlx::Result<()> {
    let (created, _credential_names) =
        create_connector(&pool, &minimal_input("rest basic auth one secret"))
            .await
            .unwrap();
    let spec = IngestSpecInput {
        adapter: "rest".to_owned(),
        ingest_mode: "batch".to_owned(),
        dial: serde_json::json!({
            "baseUrl": "https://api.example.internal",
            "auth": {"type": "basic"},
            "pagination": {"type": "none"},
            "endpoints": [],
        }),
        source_objects: serde_json::json!([]),
        schedule_cron: None,
    };
    let err = set_ingest_spec(&pool, &created.id, &spec)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::Validation(_)));

    // The rejected spec must never have reached the row.
    let read = get_ingest_spec(&pool, &created.id).await.unwrap().unwrap();
    assert_eq!(
        read.adapter, None,
        "a spec whose secret-ref count doesn't match its adapter/auth combination must never be persisted"
    );
    Ok(())
}

/// The positive control for the test above: a `rest`/`basic` connector
/// created WITH both secret refs set is accepted.
#[sqlx::test(migrations = "../../migrations")]
async fn set_ingest_spec_accepts_basic_auth_rest_with_two_secret_refs(
    pool: PgPool,
) -> sqlx::Result<()> {
    let (created, _credential_names) = create_connector(
        &pool,
        &CreateConnectorInput {
            credential: two_slot_credential(),
            ..minimal_input("rest basic auth two secrets")
        },
    )
    .await
    .unwrap();
    let spec = IngestSpecInput {
        adapter: "rest".to_owned(),
        ingest_mode: "batch".to_owned(),
        dial: serde_json::json!({
            "baseUrl": "https://api.example.internal",
            "auth": {"type": "basic"},
            "pagination": {"type": "none"},
            "endpoints": [],
        }),
        source_objects: serde_json::json!([]),
        schedule_cron: None,
    };
    set_ingest_spec(&pool, &created.id, &spec).await.unwrap();

    let read = get_ingest_spec(&pool, &created.id).await.unwrap().unwrap();
    assert_eq!(read.adapter.as_deref(), Some("rest"));
    Ok(())
}

/// A `sheets` adapter only ever needs ONE secret ref
/// (`secret_field_names("sheets", None) == ("serviceAccountJson",)`), so
/// `minimal_input`'s single `secret_ref` (no secondary) is sufficient —
/// this is the "the validation two tests above does not over-reject the
/// common case" check.
#[sqlx::test(migrations = "../../migrations")]
async fn set_ingest_spec_accepts_sheets_adapter_with_one_secret_ref(
    pool: PgPool,
) -> sqlx::Result<()> {
    let (created, _credential_names) =
        create_connector(&pool, &minimal_input("sheets adapter one secret"))
            .await
            .unwrap();
    let spec = IngestSpecInput {
        adapter: "sheets".to_owned(),
        ingest_mode: "batch".to_owned(),
        dial: serde_json::json!({"spreadsheetId": "abc123", "ranges": ["Sheet1!A1:B2"]}),
        source_objects: serde_json::json!([]),
        schedule_cron: None,
    };
    set_ingest_spec(&pool, &created.id, &spec).await.unwrap();
    Ok(())
}

/// A `files` adapter needs TWO secret refs
/// (`secret_field_names("files", None) == ("accessKey", "secretKey")`);
/// `minimal_input` alone (one `secretRef`, no secondary) must be
/// rejected, complementing the `rest`/`basic` case tested above with a
/// second adapter that also needs two.
#[sqlx::test(migrations = "../../migrations")]
async fn set_ingest_spec_rejects_files_adapter_with_only_one_secret_ref(
    pool: PgPool,
) -> sqlx::Result<()> {
    let (created, _credential_names) =
        create_connector(&pool, &minimal_input("files adapter one secret rejected"))
            .await
            .unwrap();
    let spec = IngestSpecInput {
        adapter: "files".to_owned(),
        ingest_mode: "batch".to_owned(),
        dial: serde_json::json!({"protocol": "s3", "bucket": "b", "format": "csv"}),
        source_objects: serde_json::json!([]),
        schedule_cron: None,
    };
    let err = set_ingest_spec(&pool, &created.id, &spec)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::Validation(_)));
    Ok(())
}

/// The positive control: a `files` connector created WITH both secret
/// refs set is accepted.
#[sqlx::test(migrations = "../../migrations")]
async fn set_ingest_spec_accepts_files_adapter_with_two_secret_refs(
    pool: PgPool,
) -> sqlx::Result<()> {
    let (created, _credential_names) = create_connector(
        &pool,
        &CreateConnectorInput {
            credential: two_slot_credential(),
            ..minimal_input("files adapter two secrets")
        },
    )
    .await
    .unwrap();
    let spec = IngestSpecInput {
        adapter: "files".to_owned(),
        ingest_mode: "batch".to_owned(),
        dial: serde_json::json!({"protocol": "s3", "bucket": "b", "format": "csv"}),
        source_objects: serde_json::json!([]),
        schedule_cron: None,
    };
    set_ingest_spec(&pool, &created.id, &spec).await.unwrap();
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

/// Minimal valid input for [`create_tenant`], varying only `slug` (unique)
/// — matches `lakehouse_store::identity::CreateTenantInput`'s shape.
fn tenant_input(slug: &str) -> CreateTenantInput {
    CreateTenantInput {
        name: slug.to_owned(),
        slug: slug.to_owned(),
        plan: "starter".to_owned(),
        residency: "in-region".to_owned(),
    }
}

/// Assign a connector to a tenant. Written before
/// `assign_tenant` (this module's own store function for that write)
/// existed — a
/// direct, bound `UPDATE` is the only way this test can set up a
/// tenant-scoped fixture today, matching `0042_tenant_provisioning.sql`'s
/// own column exactly (nullable `connector.tenant_id`).
async fn set_connector_tenant(pool: &PgPool, connector_id: &str, tenant_id: Uuid) {
    sqlx::query("UPDATE connector SET tenant_id = $1 WHERE id = $2")
        .bind(tenant_id)
        .bind(connector_id)
        .execute(pool)
        .await
        .unwrap();
}

/// Written and run BEFORE `ConnectorFilter`
/// existed, as a failing test first. The real failure this produced
/// (`ConnectorFilter` stripped, `list_connectors` reverted to its
/// one-arg signature):
///
/// ```text
/// error[E0433]: failed to resolve: could not find `ConnectorFilter` in `connectors`
/// error[E0061]: this function takes 1 argument but 2 arguments were supplied
/// ```
///
/// Tenant isolation requires asserting a specific second tenant's row
/// ABSENT, never merely "the list is shorter" or "non-empty."
#[sqlx::test(migrations = "../../migrations")]
async fn list_connectors_filtered_by_tenant_excludes_another_tenants_row(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = create_tenant(&pool, &tenant_input("tenant-a-connectors"))
        .await
        .unwrap();
    let tenant_b = create_tenant(&pool, &tenant_input("tenant-b-connectors"))
        .await
        .unwrap();
    let tenant_a_id: Uuid = tenant_a.id.parse().unwrap();
    let tenant_b_id: Uuid = tenant_b.id.parse().unwrap();

    let (conn_a, _) = create_connector(&pool, &minimal_input("conn-a isolation"))
        .await
        .unwrap();
    let (conn_b, _) = create_connector(&pool, &minimal_input("conn-b isolation"))
        .await
        .unwrap();
    set_connector_tenant(&pool, &conn_a.id, tenant_a_id).await;
    set_connector_tenant(&pool, &conn_b.id, tenant_b_id).await;

    let rows = list_connectors(
        &pool,
        &ConnectorFilter {
            tenant_id: Some(tenant_a_id),
        },
    )
    .await
    .unwrap();
    assert!(
        rows.iter().any(|r| r.id == conn_a.id),
        "tenant-a's own connector must be present"
    );
    assert!(
        !rows.iter().any(|r| r.id == conn_b.id),
        "tenant-b's connector must be absent, not merely unlisted-first"
    );
    Ok(())
}

// ── connector_probe_result: per-connector probe history ────────────────

fn probe_history_test_input(name: &str) -> CreateConnectorInput {
    CreateConnectorInput {
        name: name.to_owned(),
        kind: "REST API".to_owned(),
        direction: "source".to_owned(),
        host: "h".to_owned(),
        credential: single_credential(),
        environment: "staging".to_owned(),
        tenant: "Meridian Group".to_owned(),
        residency: String::new(),
        capabilities: vec![],
        owner: None,
    }
}

/// A supported probe writes exactly one history row whose `ok`/
/// `latency_ms`/`message` match what was measured, and whose `tested_at`
/// equals the `tested_at` `record_test_result` itself returns -- proving
/// the current-state `UPDATE` and the history insert share the exact same
/// timestamp rather than each calling `now()` independently.
#[sqlx::test(migrations = "../../migrations")]
async fn supported_probe_writes_exactly_one_history_row(pool: PgPool) -> sqlx::Result<()> {
    let (created, _credential_names) =
        create_connector(&pool, &probe_history_test_input("history: supported"))
            .await
            .unwrap();

    let result = record_test_result(&pool, &created.id, true, true, Some(37), "ok, real dial")
        .await
        .unwrap();

    let history = list_probe_results(&pool, &created.id, 50).await.unwrap();
    assert_eq!(history.len(), 1);
    assert!(history[0].ok);
    assert_eq!(history[0].latency_ms, Some(37));
    assert_eq!(history[0].message, "ok, real dial");
    assert_eq!(
        Some(history[0].tested_at.clone()),
        result.tested_at,
        "the history row's tested_at must equal the value record_test_result returned"
    );
    Ok(())
}

/// Fail closed: a connector whose `tenant_id` column is `NULL`
/// (unassigned) must never appear in ANY tenant-scoped list —
/// `0042_tenant_provisioning.sql`'s own stated contract.
#[sqlx::test(migrations = "../../migrations")]
async fn list_connectors_with_a_null_tenant_id_is_invisible_once_scoped(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = create_tenant(&pool, &tenant_input("tenant-a-null-connector"))
        .await
        .unwrap();
    let tenant_a_id: Uuid = tenant_a.id.parse().unwrap();

    // Freshly created via `create_connector`, never assigned a tenant —
    // `tenant_id` stays the column's default, `NULL`.
    create_connector(&pool, &minimal_input("unassigned connector"))
        .await
        .unwrap();

    let rows = list_connectors(
        &pool,
        &ConnectorFilter {
            tenant_id: Some(tenant_a_id),
        },
    )
    .await
    .unwrap();
    assert!(
        rows.is_empty(),
        "a connector with tenant_id NULL must not leak into any tenant's scoped list"
    );
    Ok(())
}

/// An unsupported probe writes NO history row -- an unsupported probe
/// never actually dialed the connector, so it has no outcome to record
/// (same rule `connector.health`/`lastTestAt` already follow).
#[sqlx::test(migrations = "../../migrations")]
async fn unsupported_probe_writes_no_history_row(pool: PgPool) -> sqlx::Result<()> {
    let (created, _credential_names) =
        create_connector(&pool, &probe_history_test_input("history: unsupported"))
            .await
            .unwrap();

    record_test_result(&pool, &created.id, false, false, None, "unsupported")
        .await
        .unwrap();

    let history = list_probe_results(&pool, &created.id, 50).await.unwrap();
    assert!(
        history.is_empty(),
        "an unsupported probe must never write a history row"
    );
    Ok(())
}

/// Testing an unknown connector id writes nothing and returns `NotFound` --
/// exactly `record_test_result`'s existing contract for the current-state
/// `UPDATE`, now also true for the history insert.
#[sqlx::test(migrations = "../../migrations")]
async fn unknown_connector_probe_writes_nothing(pool: PgPool) -> sqlx::Result<()> {
    let err = record_test_result(&pool, "conn-does-not-exist", true, true, Some(1), "x")
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::NotFound));

    let history = list_probe_results(&pool, "conn-does-not-exist", 50)
        .await
        .unwrap();
    assert!(history.is_empty());
    Ok(())
}

/// 205 supported probes leave exactly 200 rows, and the survivors are the
/// newest 200 -- `insert_and_trim`'s per-connector cap.
#[sqlx::test(migrations = "../../migrations")]
async fn probe_history_is_trimmed_to_the_newest_two_hundred(pool: PgPool) -> sqlx::Result<()> {
    let (created, _credential_names) =
        create_connector(&pool, &probe_history_test_input("history: trim"))
            .await
            .unwrap();

    for i in 0..205 {
        record_test_result(
            &pool,
            &created.id,
            true,
            true,
            Some(i),
            &format!("probe {i}"),
        )
        .await
        .unwrap();
    }

    let history = list_probe_results(&pool, &created.id, 500).await.unwrap();
    assert_eq!(history.len(), 200);
    // Newest first: the very first probe recorded ("probe 0".."probe 4")
    // must have been trimmed away, and the last one recorded ("probe 204")
    // must be the newest (first) entry.
    assert_eq!(history[0].message, "probe 204");
    assert_eq!(history[199].message, "probe 5");
    for row in &history {
        assert_ne!(row.message, "probe 0");
        assert_ne!(row.message, "probe 4");
    }
    Ok(())
}

/// `list_probe_results` returns newest first and honours `limit`.
#[sqlx::test(migrations = "../../migrations")]
async fn list_probe_results_returns_newest_first_and_honours_limit(
    pool: PgPool,
) -> sqlx::Result<()> {
    let (created, _credential_names) =
        create_connector(&pool, &probe_history_test_input("history: ordering"))
            .await
            .unwrap();

    for i in 0..5 {
        record_test_result(
            &pool,
            &created.id,
            true,
            true,
            Some(i),
            &format!("probe {i}"),
        )
        .await
        .unwrap();
    }

    let all = list_probe_results(&pool, &created.id, 50).await.unwrap();
    assert_eq!(all.len(), 5);
    assert_eq!(all[0].message, "probe 4");
    assert_eq!(all[4].message, "probe 0");

    let limited = list_probe_results(&pool, &created.id, 2).await.unwrap();
    assert_eq!(limited.len(), 2);
    assert_eq!(limited[0].message, "probe 4");
    assert_eq!(limited[1].message, "probe 3");
    Ok(())
}

/// Deleting a connector cascades its probe history away
/// (`connector_id ... REFERENCES connector(id) ON DELETE CASCADE`,
/// `0044_connector_probe_result.sql`).
#[sqlx::test(migrations = "../../migrations")]
async fn deleting_a_connector_cascades_its_probe_history(pool: PgPool) -> sqlx::Result<()> {
    let (created, _credential_names) =
        create_connector(&pool, &probe_history_test_input("history: cascade"))
            .await
            .unwrap();
    record_test_result(&pool, &created.id, true, true, Some(1), "ok")
        .await
        .unwrap();
    assert_eq!(
        list_probe_results(&pool, &created.id, 50)
            .await
            .unwrap()
            .len(),
        1
    );

    assert!(delete_connector(&pool, &created.id).await.unwrap());

    let history = list_probe_results(&pool, &created.id, 50).await.unwrap();
    assert!(
        history.is_empty(),
        "deleting the connector must cascade-delete its probe history"
    );
    Ok(())
}

// ---- `swap_secret_ref` ----

/// The success path: `expected_old` matches the connector's current
/// primary `secret_ref`, so the swap lands and the new value round-trips
/// back out through `get_connector_dial_info`.
#[sqlx::test(migrations = "../../migrations")]
async fn swap_secret_ref_rotates_the_primary_slot_when_expected_old_matches(
    pool: PgPool,
) -> sqlx::Result<()> {
    let (created, _credential_names) =
        create_connector(&pool, &minimal_input("rotate primary target"))
            .await
            .unwrap();
    let before = get_connector_dial_info(&pool, &created.id)
        .await
        .unwrap()
        .unwrap();

    swap_secret_ref(
        &pool,
        &created.id,
        SecretSlot::Primary,
        Some(&before.secret_ref),
        "env:ROTATED_PRIMARY_REF",
    )
    .await
    .unwrap();

    let after = get_connector_dial_info(&pool, &created.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.secret_ref, "env:ROTATED_PRIMARY_REF");
    Ok(())
}

/// A stale `expected_old` (not the connector's actual current ref) must
/// be refused as a [`StoreError::Conflict`], and the value on the row
/// must be left exactly as it was -- the compare-and-swap's whole
/// purpose.
#[sqlx::test(migrations = "../../migrations")]
async fn swap_secret_ref_with_a_stale_expected_old_is_a_conflict_and_leaves_the_value_unchanged(
    pool: PgPool,
) -> sqlx::Result<()> {
    let (created, credential_names) =
        create_connector(&pool, &minimal_input("rotate stale target"))
            .await
            .unwrap();

    let err = swap_secret_ref(
        &pool,
        &created.id,
        SecretSlot::Primary,
        Some("env:THIS_IS_NOT_THE_CURRENT_REF"),
        "env:WOULD_BE_NEW_REF",
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict),
        "expected Conflict, got {err:?}"
    );

    let after = get_connector_dial_info(&pool, &created.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        after.secret_ref, credential_names.primary,
        "a failed swap must never touch the stored value"
    );
    Ok(())
}

/// An unknown connector id is [`StoreError::NotFound`], not
/// [`StoreError::Conflict`] -- the zero-rows disambiguation this function's
/// doc comment describes.
#[sqlx::test(migrations = "../../migrations")]
async fn swap_secret_ref_on_an_unknown_id_is_not_found(pool: PgPool) -> sqlx::Result<()> {
    let err = swap_secret_ref(
        &pool,
        "conn-does-not-exist-at-all",
        SecretSlot::Primary,
        Some("env:ANYTHING"),
        "env:NEW_REF",
    )
    .await
    .unwrap_err();
    assert!(matches!(err, StoreError::NotFound), "got {err:?}");
    Ok(())
}

/// A connector whose secondary slot has never been set (`NULL`) can still
/// be rotated by passing `expected_old: None` -- `IS NOT DISTINCT FROM`
/// (rather than `=`) is what makes a `NULL`-to-`NULL` comparison match.
#[sqlx::test(migrations = "../../migrations")]
async fn swap_secret_ref_sets_a_null_secondary_slot_when_expected_old_is_none(
    pool: PgPool,
) -> sqlx::Result<()> {
    let (created, _credential_names) =
        create_connector(&pool, &minimal_input("rotate secondary target"))
            .await
            .unwrap();
    let before = get_connector_dial_info(&pool, &created.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        before.secret_ref_secondary, None,
        "minimal_input leaves secret_ref_secondary unset"
    );

    swap_secret_ref(
        &pool,
        &created.id,
        SecretSlot::Secondary,
        None,
        "env:NEW_SECONDARY_REF",
    )
    .await
    .unwrap();

    let after = get_connector_dial_info(&pool, &created.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        after.secret_ref_secondary.as_deref(),
        Some("env:NEW_SECONDARY_REF")
    );
    Ok(())
}
