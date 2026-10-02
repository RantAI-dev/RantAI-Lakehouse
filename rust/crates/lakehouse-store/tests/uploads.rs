//! Integration tests for `lakehouse_store::uploads` and the `file_upload`
//! table (`0054_upload.sql`, `0055_upload_tenant_mode.sql`) against a real
//! Postgres.
//!
//! # Postgres backing
//!
//! These are `#[sqlx::test(migrations = "../../migrations")]` tests: each
//! one gets a freshly migrated, isolated database, which is also how
//! `0055` is proven to apply on a fresh one. The Postgres *server* itself
//! is started once per test binary by the `lakehouse-test-support`
//! dev-dependency (see `tests/connectors.rs`). The two migration tests
//! below start from an EMPTY database and apply `0054` first, then the
//! rest, to prove `0055` applies on a database that already has `0054`.
//!
//! `any_connector_targets` (the other store function T3 adds) is tested in
//! `tests/connectors.rs`, beside the connector helpers it needs.

#![allow(clippy::unwrap_used, clippy::expect_used)]

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap actually runs for this test binary.
use lakehouse_test_support as _;

use std::borrow::Cow;

use lakehouse_store::StoreError;
use lakehouse_store::identity::{CreateTenantInput, create_tenant};
use lakehouse_store::uploads::{
    LoadMode, NewUpload, Upload, delete, find_by_sha256, get, insert, list, mark_finished,
    mark_ingesting, table_being_loaded, table_created_by_upload, upload_in_tenants,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::migrate::Migrator;
use uuid::Uuid;

/// The embedded migration set, to apply in two steps in the migration
/// tests.
static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

async fn tenant(pool: &PgPool, slug: &str) -> Uuid {
    let created = create_tenant(
        pool,
        &CreateTenantInput {
            name: slug.to_owned(),
            slug: slug.to_owned(),
            plan: "starter".to_owned(),
            residency: "in-region".to_owned(),
        },
    )
    .await
    .unwrap();
    created.id.parse().unwrap()
}

/// Record an upload of `tenant_id` the way the create route does, with a
/// key built from the id so two uploads never share one.
async fn add(pool: &PgPool, id: &str, tenant_id: Uuid, sha256: &str) -> Upload {
    let key = format!("uploads/{tenant_id}/{id}.csv");
    insert(
        pool,
        &NewUpload {
            id,
            original_filename: "stock.csv",
            storage_key: &key,
            content_type: "text/csv",
            size_bytes: 2048,
            sha256,
            uploaded_by: "Test User",
            tenant_id,
        },
    )
    .await
    .unwrap()
}

fn options() -> Value {
    json!({ "encoding": "utf-8", "delimiter": ",", "headerRow": 0 })
}

/// Start a load of `id` into `table`; the row must be loadable.
async fn start(
    pool: &PgPool,
    id: &str,
    table: &str,
    mode: LoadMode,
    run_id: Option<&str>,
) -> Upload {
    mark_ingesting(pool, id, &options(), table, mode, run_id)
        .await
        .unwrap()
        .expect("the upload is loadable")
}

async fn set_created_at(pool: &PgPool, id: &str, hours_ago: i32) {
    sqlx::query(
        "UPDATE file_upload SET created_at = now() - make_interval(hours => $2) WHERE id = $1",
    )
    .bind(id)
    .bind(hours_ago)
    .execute(pool)
    .await
    .unwrap();
}

fn ids(rows: &[Upload]) -> Vec<&str> {
    rows.iter().map(|r| r.id.as_str()).collect()
}

#[sqlx::test(migrations = "../../migrations")]
async fn insert_stores_the_tenant_and_starts_in_the_uploaded_state(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-insert-a").await;
    let up = add(&pool, "up-1", tenant_a, &"a".repeat(64)).await;

    assert_eq!(up.tenant_id, Some(tenant_a));
    assert_eq!(up.status, "uploaded");
    assert_eq!(up.original_filename, "stock.csv");
    assert_eq!(up.size_bytes, 2048);
    assert_eq!(up.storage_key, format!("uploads/{tenant_a}/up-1.csv"));
    assert!(up.parse_options.is_none());
    assert!(up.bronze_table.is_none());
    assert!(up.load_mode.is_none());
    assert!(up.rows.is_none(), "an upload never loaded has no row count");
    assert!(up.run_id.is_none());
    assert!(up.error.is_none());
    Ok(())
}

/// A key that is already registered is the unique violation `insert`
/// documents; so is a repeated id.
#[sqlx::test(migrations = "../../migrations")]
async fn insert_rejects_a_repeated_storage_key_or_id_as_a_conflict(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-conflict").await;
    let first = add(&pool, "up-1", tenant_a, "").await;

    let same_key = insert(
        &pool,
        &NewUpload {
            id: "up-2",
            original_filename: "other.csv",
            storage_key: &first.storage_key,
            content_type: "",
            size_bytes: 1,
            sha256: "",
            uploaded_by: "Test User",
            tenant_id: tenant_a,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(same_key, StoreError::Conflict), "{same_key:?}");

    let same_id = insert(
        &pool,
        &NewUpload {
            id: "up-1",
            original_filename: "other.csv",
            storage_key: "uploads/elsewhere/up-1.csv",
            content_type: "",
            size_bytes: 1,
            sha256: "",
            uploaded_by: "Test User",
            tenant_id: tenant_a,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(same_id, StoreError::Conflict), "{same_id:?}");
    Ok(())
}

/// `tenant_id` is a foreign key: a tenant that does not exist cannot own an
/// upload, so a handler that passed the wrong id cannot create a row nobody
/// can ever see.
#[sqlx::test(migrations = "../../migrations")]
async fn insert_rejects_an_unknown_tenant_as_a_foreign_key_violation(
    pool: PgPool,
) -> sqlx::Result<()> {
    let err = insert(
        &pool,
        &NewUpload {
            id: "up-1",
            original_filename: "stock.csv",
            storage_key: "uploads/nobody/up-1.csv",
            content_type: "",
            size_bytes: 1,
            sha256: "",
            uploaded_by: "Test User",
            tenant_id: Uuid::new_v4(),
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, StoreError::ForeignKeyViolation), "{err:?}");
    Ok(())
}

/// What a response built from an `Upload` carries: no object key and no
/// tenant, and no `rows` or `loadMode` until a load has set them (a count
/// nobody measured must not read as 0).
#[sqlx::test(migrations = "../../migrations")]
async fn the_serialized_upload_hides_the_key_and_the_tenant_and_omits_what_is_not_set(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-serialize").await;
    let up = add(&pool, "up-1", tenant_a, &"b".repeat(64)).await;
    let body = serde_json::to_value(&up).unwrap();

    for hidden in ["storageKey", "tenantId", "tenant"] {
        assert!(
            body.get(hidden).is_none(),
            "{hidden} must not be serialized"
        );
    }
    let raw = body.to_string();
    assert!(!raw.contains("uploads/"), "no object key in {raw}");
    assert!(
        !raw.contains(&tenant_a.to_string()),
        "no tenant id in {raw}"
    );
    for shown in [
        "id",
        "originalFilename",
        "contentType",
        "sizeBytes",
        "sha256",
        "uploadedBy",
        "status",
        "createdAt",
        "updatedAt",
    ] {
        assert!(body.get(shown).is_some(), "{shown} must be serialized");
    }
    assert!(body["createdAt"].as_str().unwrap().ends_with('Z'));
    for unset in [
        "parseOptions",
        "bronzeTable",
        "loadMode",
        "rows",
        "runId",
        "error",
    ] {
        assert!(body.get(unset).is_none(), "{unset} is absent until set");
    }

    // After a load: the options, the table, the mode and the count are
    // there, under their camelCase names.
    start(&pool, "up-1", "stock_raw", LoadMode::Append, Some("run-1")).await;
    let done = mark_finished(&pool, "up-1", Some("run-1"), None, Some(42))
        .await
        .unwrap()
        .unwrap();
    let body = serde_json::to_value(&done).unwrap();
    assert_eq!(body["status"], "ingested");
    assert_eq!(body["bronzeTable"], "stock_raw");
    assert_eq!(body["loadMode"], "append");
    assert_eq!(body["rows"], 42);
    assert_eq!(body["runId"], "run-1");
    assert_eq!(body["parseOptions"]["delimiter"], ",");
    assert!(body.get("error").is_none());

    // A load whose sink reported no total: `rows` is absent, never 0.
    start(&pool, "up-1", "stock_raw", LoadMode::Replace, Some("run-2")).await;
    let unmeasured = mark_finished(&pool, "up-1", Some("run-2"), None, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unmeasured.status, "ingested");
    assert_eq!(unmeasured.rows, None);
    assert!(
        serde_json::to_value(&unmeasured)
            .unwrap()
            .get("rows")
            .is_none()
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn list_returns_only_the_callers_tenant_newest_first(pool: PgPool) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-list-a").await;
    let tenant_b = tenant(&pool, "uploads-list-b").await;
    let tenant_c = tenant(&pool, "uploads-list-c").await;
    for (id, hours_ago) in [("up-a1", 3), ("up-a2", 2), ("up-a3", 1)] {
        add(&pool, id, tenant_a, "").await;
        set_created_at(&pool, id, hours_ago).await;
    }
    add(&pool, "up-b1", tenant_b, "").await;

    let rows = list(&pool, tenant_a, 100).await.unwrap();
    assert_eq!(ids(&rows), ["up-a3", "up-a2", "up-a1"]);
    assert!(
        !rows.iter().any(|r| r.id == "up-b1"),
        "another tenant's upload must be absent, not merely listed last"
    );
    assert_eq!(ids(&list(&pool, tenant_b, 100).await.unwrap()), ["up-b1"]);
    assert!(
        list(&pool, tenant_c, 100).await.unwrap().is_empty(),
        "a tenant with no uploads gets an empty list"
    );

    assert_eq!(
        ids(&list(&pool, tenant_a, 2).await.unwrap()),
        ["up-a3", "up-a2"]
    );
    assert_eq!(
        list(&pool, tenant_a, 0).await.unwrap().len(),
        1,
        "a limit below 1 is clamped to 1"
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn get_returns_the_row_or_none(pool: PgPool) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-get").await;
    add(&pool, "up-1", tenant_a, "").await;

    let found = get(&pool, "up-1").await.unwrap().unwrap();
    assert_eq!(found.id, "up-1");
    assert!(get(&pool, "up-missing").await.unwrap().is_none());
    Ok(())
}

/// The per-id access rule: the upload's own tenant is in; another tenant,
/// no tenant at all, an upload with no tenant and an unknown id are all out.
#[sqlx::test(migrations = "../../migrations")]
async fn upload_in_tenants_is_false_for_another_tenant_for_no_tenant_and_for_an_unknown_id(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-access-a").await;
    let tenant_b = tenant(&pool, "uploads-access-b").await;
    add(&pool, "up-1", tenant_a, "").await;

    assert!(upload_in_tenants(&pool, "up-1", &[tenant_a]).await.unwrap());
    assert!(
        upload_in_tenants(&pool, "up-1", &[tenant_b, tenant_a])
            .await
            .unwrap()
    );
    assert!(!upload_in_tenants(&pool, "up-1", &[tenant_b]).await.unwrap());
    assert!(!upload_in_tenants(&pool, "up-1", &[]).await.unwrap());
    assert!(
        !upload_in_tenants(&pool, "up-unknown", &[tenant_a, tenant_b])
            .await
            .unwrap()
    );

    sqlx::query("UPDATE file_upload SET tenant_id = NULL WHERE id = 'up-1'")
        .execute(&pool)
        .await?;
    assert!(
        !upload_in_tenants(&pool, "up-1", &[tenant_a, tenant_b])
            .await
            .unwrap(),
        "an upload with no tenant belongs to nobody"
    );
    Ok(())
}

/// Duplicate lookup is per tenant: the same bytes in another tenant are not
/// reported, the asking upload never matches itself, and an empty checksum
/// matches nothing.
#[sqlx::test(migrations = "../../migrations")]
async fn find_by_sha256_sees_only_its_own_tenant_and_never_the_excluded_upload(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-dup-a").await;
    let tenant_b = tenant(&pool, "uploads-dup-b").await;
    let sha = "c".repeat(64);
    add(&pool, "up-1", tenant_a, &sha).await;
    set_created_at(&pool, "up-1", 2).await;
    add(&pool, "up-2", tenant_a, &sha).await;
    add(&pool, "up-3", tenant_b, &sha).await;

    let earlier = find_by_sha256(&pool, tenant_a, &sha, "up-2")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(earlier.id, "up-1", "the asking upload is excluded");

    let newest = find_by_sha256(&pool, tenant_a, &sha, "up-unrelated")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(newest.id, "up-2", "the newest match wins");

    assert!(
        find_by_sha256(&pool, tenant_b, &sha, "up-3")
            .await
            .unwrap()
            .is_none(),
        "tenant A's uploads are invisible to tenant B"
    );
    let own = find_by_sha256(&pool, tenant_b, &sha, "up-unrelated")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(own.id, "up-3");

    add(&pool, "up-4", tenant_a, "").await;
    assert!(
        find_by_sha256(&pool, tenant_a, "", "up-unrelated")
            .await
            .unwrap()
            .is_none(),
        "an empty checksum is no checksum"
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn mark_ingesting_records_the_options_the_table_the_mode_and_the_run(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-ingesting").await;
    let before = add(&pool, "up-1", tenant_a, "").await;

    let opts = json!({ "encoding": "utf-16", "delimiter": "\t", "headerRow": 4 });
    let after = mark_ingesting(
        &pool,
        "up-1",
        &opts,
        "stock_raw",
        LoadMode::Append,
        Some("run-1"),
    )
    .await
    .unwrap()
    .unwrap();

    assert_eq!(after.status, "ingesting");
    assert_eq!(after.parse_options.as_ref().map(|o| &o.0), Some(&opts));
    assert_eq!(after.bronze_table.as_deref(), Some("stock_raw"));
    assert_eq!(after.load_mode.as_deref(), Some("append"));
    assert_eq!(after.run_id.as_deref(), Some("run-1"));
    assert!(after.error.is_none());
    assert!(after.rows.is_none());
    assert!(after.updated_at > before.updated_at);
    assert_eq!(after.created_at, before.created_at);

    assert!(
        mark_ingesting(
            &pool,
            "up-missing",
            &opts,
            "stock_raw",
            LoadMode::Replace,
            None
        )
        .await
        .unwrap()
        .is_none(),
        "an unknown upload changes nothing"
    );
    Ok(())
}

/// One load per upload at a time, enforced where the row changes: a second
/// request cannot replace the run and the options of a load in flight.
#[sqlx::test(migrations = "../../migrations")]
async fn mark_ingesting_refuses_an_upload_that_is_already_loading(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-busy").await;
    add(&pool, "up-1", tenant_a, "").await;
    start(&pool, "up-1", "stock_raw", LoadMode::Replace, Some("run-1")).await;

    let second = mark_ingesting(
        &pool,
        "up-1",
        &json!({ "delimiter": ";" }),
        "other_raw",
        LoadMode::Append,
        Some("run-2"),
    )
    .await
    .unwrap();
    assert!(second.is_none());

    let row = get(&pool, "up-1").await.unwrap().unwrap();
    assert_eq!(row.run_id.as_deref(), Some("run-1"));
    assert_eq!(row.bronze_table.as_deref(), Some("stock_raw"));
    assert_eq!(row.load_mode.as_deref(), Some("replace"));
    assert_eq!(row.parse_options.map(|o| o.0), Some(options()));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn mark_finished_without_an_error_ingests_and_keeps_the_count_it_was_given(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-finish-ok").await;
    add(&pool, "up-1", tenant_a, "").await;
    start(&pool, "up-1", "stock_raw", LoadMode::Replace, Some("run-1")).await;

    let done = mark_finished(&pool, "up-1", Some("run-1"), None, Some(1234))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.status, "ingested");
    assert_eq!(done.rows, Some(1234));
    assert!(done.error.is_none());
    assert_eq!(done.bronze_table.as_deref(), Some("stock_raw"));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn mark_finished_with_an_error_fails_and_stores_no_row_count(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-finish-err").await;
    add(&pool, "up-1", tenant_a, "").await;
    start(&pool, "up-1", "stock_raw", LoadMode::Replace, Some("run-1")).await;

    // A count handed in with an error is dropped: a failed load measured
    // nothing a console could show.
    let failed = mark_finished(
        &pool,
        "up-1",
        Some("run-1"),
        Some("The load into the table failed."),
        Some(99),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(failed.status, "failed");
    assert_eq!(
        failed.error.as_deref(),
        Some("The load into the table failed.")
    );
    assert_eq!(failed.rows, None);
    Ok(())
}

/// A settle names the run it is about. A reader that learned the outcome of
/// an earlier run, or one that lost the race to settle the same run, changes
/// nothing, and a row that is not loading cannot be "finished".
#[sqlx::test(migrations = "../../migrations")]
async fn mark_finished_settles_only_the_loading_run_it_was_asked_about(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-finish-guard").await;
    add(&pool, "up-1", tenant_a, "").await;

    assert!(
        mark_finished(&pool, "up-1", None, None, Some(1))
            .await
            .unwrap()
            .is_none(),
        "an upload that is not loading cannot be finished"
    );
    assert_eq!(
        get(&pool, "up-1").await.unwrap().unwrap().status,
        "uploaded"
    );

    start(&pool, "up-1", "stock_raw", LoadMode::Replace, Some("run-2")).await;
    assert!(
        mark_finished(&pool, "up-1", Some("run-1"), None, Some(1))
            .await
            .unwrap()
            .is_none(),
        "the outcome of another run does not settle this one"
    );
    assert!(
        mark_finished(&pool, "up-1", None, None, Some(1))
            .await
            .unwrap()
            .is_none(),
        "a missing run id is not a wildcard"
    );
    assert_eq!(
        get(&pool, "up-1").await.unwrap().unwrap().status,
        "ingesting"
    );

    let done = mark_finished(&pool, "up-1", Some("run-2"), None, Some(5))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.status, "ingested");

    assert!(
        mark_finished(&pool, "up-1", Some("run-2"), Some("late"), None)
            .await
            .unwrap()
            .is_none(),
        "a second reader of the same run changes nothing"
    );
    let row = get(&pool, "up-1").await.unwrap().unwrap();
    assert_eq!((row.status.as_str(), row.rows), ("ingested", Some(5)));
    assert!(row.error.is_none());
    Ok(())
}

/// A load that started with no run id (the launch reported none) is settled
/// by the same absence, and by nothing else.
#[sqlx::test(migrations = "../../migrations")]
async fn mark_finished_matches_a_missing_run_id_only_with_a_missing_run_id(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-finish-norun").await;
    add(&pool, "up-1", tenant_a, "").await;
    start(&pool, "up-1", "stock_raw", LoadMode::Replace, None).await;

    assert!(
        mark_finished(&pool, "up-1", Some("run-1"), None, None)
            .await
            .unwrap()
            .is_none()
    );
    let done = mark_finished(&pool, "up-1", None, None, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.status, "ingested");
    Ok(())
}

/// The transitions the console offers again after a result: `ingested` and
/// `failed` both go back to `ingesting`, and the new attempt starts with no
/// reason and no count of the old one.
#[sqlx::test(migrations = "../../migrations")]
async fn an_ingested_or_failed_upload_can_be_loaded_again_from_a_clean_slate(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-reload").await;
    add(&pool, "up-1", tenant_a, "").await;

    start(&pool, "up-1", "stock_raw", LoadMode::Replace, Some("run-1")).await;
    let done = mark_finished(&pool, "up-1", Some("run-1"), None, Some(10))
        .await
        .unwrap()
        .unwrap();
    assert_eq!((done.status.as_str(), done.rows), ("ingested", Some(10)));

    let again = start(&pool, "up-1", "stock_raw", LoadMode::Append, Some("run-2")).await;
    assert_eq!(again.status, "ingesting");
    assert_eq!(again.rows, None, "the last load's count is not this one's");
    assert_eq!(again.load_mode.as_deref(), Some("append"));
    assert_eq!(again.run_id.as_deref(), Some("run-2"));

    let failed = mark_finished(
        &pool,
        "up-1",
        Some("run-2"),
        Some("The file has no rows below the header row."),
        None,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(failed.status, "failed");

    let third = start(&pool, "up-1", "stock_raw", LoadMode::Replace, Some("run-3")).await;
    assert_eq!(third.status, "ingesting");
    assert!(
        third.error.is_none(),
        "the last load's reason is not this one's"
    );
    assert_eq!(third.load_mode.as_deref(), Some("replace"));
    Ok(())
}

/// What the ingest route asks before it loads into a table that exists:
/// only an `ingested` row of the SAME tenant names it.
#[sqlx::test(migrations = "../../migrations")]
async fn table_created_by_upload_is_true_only_for_an_ingested_row_of_that_tenant(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-owner-a").await;
    let tenant_b = tenant(&pool, "uploads-owner-b").await;
    add(&pool, "up-1", tenant_a, "").await;

    assert!(
        !table_created_by_upload(&pool, tenant_a, "stock_raw")
            .await
            .unwrap(),
        "an upload that names no table created none"
    );
    start(&pool, "up-1", "stock_raw", LoadMode::Replace, Some("run-1")).await;
    assert!(
        !table_created_by_upload(&pool, tenant_a, "stock_raw")
            .await
            .unwrap(),
        "a load still running has not created it"
    );
    mark_finished(&pool, "up-1", Some("run-1"), None, Some(3))
        .await
        .unwrap();

    assert!(
        table_created_by_upload(&pool, tenant_a, "stock_raw")
            .await
            .unwrap()
    );
    assert!(
        !table_created_by_upload(&pool, tenant_b, "stock_raw")
            .await
            .unwrap(),
        "another tenant's upload did not create it"
    );
    assert!(
        !table_created_by_upload(&pool, tenant_a, "other_raw")
            .await
            .unwrap()
    );

    // A failed load into another name proves nothing about that name.
    add(&pool, "up-2", tenant_a, "").await;
    start(
        &pool,
        "up-2",
        "broken_raw",
        LoadMode::Replace,
        Some("run-2"),
    )
    .await;
    mark_finished(
        &pool,
        "up-2",
        Some("run-2"),
        Some("The load into the table failed."),
        None,
    )
    .await
    .unwrap();
    assert!(
        !table_created_by_upload(&pool, tenant_a, "broken_raw")
            .await
            .unwrap()
    );

    // Deleting the upload keeps the table (ADR 0014) but not the proof.
    assert!(delete(&pool, "up-1").await.unwrap());
    assert!(
        !table_created_by_upload(&pool, tenant_a, "stock_raw")
            .await
            .unwrap()
    );
    Ok(())
}

/// The other half of the table check: another upload, of any tenant, is
/// loading into the name right now. The asking upload never blocks itself.
#[sqlx::test(migrations = "../../migrations")]
async fn table_being_loaded_sees_another_upload_loading_and_never_the_asker(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-busy-a").await;
    let tenant_b = tenant(&pool, "uploads-busy-b").await;
    add(&pool, "up-a1", tenant_a, "").await;
    add(&pool, "up-a2", tenant_a, "").await;
    add(&pool, "up-b1", tenant_b, "").await;

    assert!(
        !table_being_loaded(&pool, "stock_raw", "up-a2")
            .await
            .unwrap()
    );
    start(
        &pool,
        "up-a1",
        "stock_raw",
        LoadMode::Replace,
        Some("run-1"),
    )
    .await;

    assert!(
        table_being_loaded(&pool, "stock_raw", "up-a2")
            .await
            .unwrap()
    );
    assert!(
        table_being_loaded(&pool, "stock_raw", "up-b1")
            .await
            .unwrap(),
        "table names are shared by every tenant"
    );
    assert!(
        !table_being_loaded(&pool, "stock_raw", "up-a1")
            .await
            .unwrap(),
        "an upload's own load does not block itself"
    );
    assert!(
        !table_being_loaded(&pool, "other_raw", "up-a2")
            .await
            .unwrap()
    );

    mark_finished(&pool, "up-a1", Some("run-1"), None, Some(1))
        .await
        .unwrap();
    assert!(
        !table_being_loaded(&pool, "stock_raw", "up-a2")
            .await
            .unwrap(),
        "a finished load no longer holds the name"
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn delete_reports_whether_a_row_was_removed(pool: PgPool) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-delete").await;
    add(&pool, "up-1", tenant_a, "").await;

    assert!(delete(&pool, "up-1").await.unwrap());
    assert!(!delete(&pool, "up-1").await.unwrap());
    assert!(get(&pool, "up-1").await.unwrap().is_none());
    Ok(())
}

/// `tenant_id` is `ON DELETE SET NULL`, the shape connectors have: a row
/// whose tenant is gone stays, and is invisible to every tenant-scoped read.
#[sqlx::test(migrations = "../../migrations")]
async fn deleting_a_tenant_leaves_its_uploads_without_a_tenant_and_invisible(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-orphan-a").await;
    add(&pool, "up-1", tenant_a, &"d".repeat(64)).await;

    sqlx::query("DELETE FROM tenant WHERE id = $1")
        .bind(tenant_a)
        .execute(&pool)
        .await?;

    let row = get(&pool, "up-1").await.unwrap().unwrap();
    assert_eq!(row.tenant_id, None);
    assert!(list(&pool, tenant_a, 100).await.unwrap().is_empty());
    assert!(!upload_in_tenants(&pool, "up-1", &[tenant_a]).await.unwrap());
    Ok(())
}

/// `load_mode` is `replace` or `append` in the database too, not only in the
/// Rust enum a handler goes through.
#[sqlx::test(migrations = "../../migrations")]
async fn the_database_refuses_a_load_mode_other_than_replace_or_append(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-mode-check").await;
    add(&pool, "up-1", tenant_a, "").await;

    for accepted in [LoadMode::Replace, LoadMode::Append] {
        sqlx::query("UPDATE file_upload SET load_mode = $1 WHERE id = 'up-1'")
            .bind(accepted.as_str())
            .execute(&pool)
            .await?;
    }
    let err = sqlx::query("UPDATE file_upload SET load_mode = 'merge' WHERE id = 'up-1'")
        .execute(&pool)
        .await
        .unwrap_err();
    assert!(
        err.as_database_error()
            .is_some_and(sqlx::error::DatabaseError::is_check_violation),
        "expected a CHECK violation, got {err:?}"
    );
    Ok(())
}

/// The first half of "does 0055 apply on a database that has 0054": `0054`
/// applied, no rows, then everything after it. (The other half, a fresh
/// database, is every `#[sqlx::test(migrations = ...)]` in this crate.)
#[sqlx::test(migrations = false)]
async fn migration_0055_applies_on_a_database_that_has_0054_and_no_rows(
    pool: PgPool,
) -> sqlx::Result<()> {
    migrations_up_to(54).run(&pool).await.unwrap();
    let before = file_upload_columns(&pool).await;
    assert!(before.contains(&"tenant".to_owned()));
    assert!(!before.contains(&"tenant_id".to_owned()));

    MIGRATOR.run(&pool).await.unwrap();

    let after = file_upload_columns(&pool).await;
    for added in ["tenant_id", "load_mode", "row_count"] {
        assert!(after.contains(&added.to_owned()), "{added} was not added");
    }
    assert!(
        !after.contains(&"tenant".to_owned()),
        "the free-text tenant column is gone"
    );
    Ok(())
}

/// What `0055` does to a row `0054` could in principle have held: it stays,
/// with no tenant, and every tenant-scoped read ignores it. Nothing wrote
/// such a row (the header of `0055` says why); this pins what would happen.
#[sqlx::test(migrations = false)]
async fn migration_0055_keeps_a_row_written_before_it_but_hides_it_from_every_tenant(
    pool: PgPool,
) -> sqlx::Result<()> {
    migrations_up_to(54).run(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO file_upload (id, original_filename, storage_key, tenant) \
         VALUES ('up-legacy', 'old.csv', 'uploads/legacy/up-legacy.csv', 'some-deployment')",
    )
    .execute(&pool)
    .await?;

    MIGRATOR.run(&pool).await.unwrap();

    let row = get(&pool, "up-legacy").await.unwrap().unwrap();
    assert_eq!(row.tenant_id, None);
    assert_eq!(row.status, "uploaded");
    let tenant_a = tenant(&pool, "uploads-legacy").await;
    assert!(list(&pool, tenant_a, 100).await.unwrap().is_empty());
    assert!(
        !upload_in_tenants(&pool, "up-legacy", &[tenant_a])
            .await
            .unwrap()
    );
    Ok(())
}

/// The migrations a database that stopped at `version` would have applied.
fn migrations_up_to(version: i64) -> Migrator {
    let upto: Vec<_> = MIGRATOR
        .iter()
        .filter(|m| m.version <= version)
        .cloned()
        .collect();
    Migrator {
        migrations: Cow::Owned(upto),
        ..Migrator::DEFAULT
    }
}

async fn file_upload_columns(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT column_name::text FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name = 'file_upload' \
         ORDER BY ordinal_position",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

/// `uploads::LoadMode` is the one place the wire names live.
#[test]
fn load_mode_parses_exactly_the_two_lowercase_names_and_defaults_to_replace() {
    assert_eq!(LoadMode::parse("replace"), Some(LoadMode::Replace));
    assert_eq!(LoadMode::parse("append"), Some(LoadMode::Append));
    for refused in ["", "Replace", "APPEND", " append", "merge", "incremental"] {
        assert_eq!(LoadMode::parse(refused), None, "{refused:?}");
    }
    assert_eq!(LoadMode::default(), LoadMode::Replace);
    for mode in [LoadMode::Replace, LoadMode::Append] {
        assert_eq!(LoadMode::parse(mode.as_str()), Some(mode));
    }
}
