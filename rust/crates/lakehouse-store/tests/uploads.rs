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
//!
//! T5a of the plan (review findings B1, B2 and B3) amended the store after
//! these tests were first written: deleting is soft, a table is owned by any
//! row of the tenant that names it, a load is claimed before it is launched,
//! and `content_type` and `sha256` are not on the wire. The tests for each
//! are named after what they pin.

#![allow(clippy::unwrap_used, clippy::expect_used)]

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap actually runs for this test binary.
use lakehouse_test_support as _;

use std::borrow::Cow;

use lakehouse_store::StoreError;
use lakehouse_store::identity::{CreateTenantInput, create_tenant};
use lakehouse_store::uploads::{
    LoadMode, NewUpload, Upload, attach_run, find_by_sha256, get, insert, list, mark_finished,
    mark_ingesting, soft_delete, table_being_loaded, table_claimed_by_upload,
    table_loaded_by_upload, upload_in_tenants,
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

/// `(rows with this id, of which marked deleted)`, read around the store's
/// own functions, which show live rows only.
async fn rows_and_deleted(pool: &PgPool, id: &str) -> (i64, i64) {
    sqlx::query_as("SELECT count(*), count(deleted_at) FROM file_upload WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Mark a row deleted whatever its state. `soft_delete` refuses a row that
/// is loading, so the tests that need a deleted row in another state set it
/// with SQL, the way a row could be left by something other than the store.
async fn force_deleted(pool: &PgPool, id: &str) {
    sqlx::query("UPDATE file_upload SET deleted_at = now() WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
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

/// What a response built from an `Upload` carries: no object key, tenant,
/// content type or checksum (review finding B3: the console needs none of
/// them), and no `rows` or `loadMode` until a load has set them (a count
/// nobody measured must not read as 0).
#[sqlx::test(migrations = "../../migrations")]
async fn the_serialized_upload_hides_what_the_console_does_not_need_and_omits_what_is_not_set(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-serialize").await;
    let sha = "b".repeat(64);
    let up = add(&pool, "up-1", tenant_a, &sha).await;
    assert_eq!(up.content_type, "text/csv", "still read from the row");
    assert_eq!(up.sha256, sha, "still read from the row");
    let body = serde_json::to_value(&up).unwrap();

    for hidden in [
        "storageKey",
        "tenantId",
        "tenant",
        "contentType",
        "sha256",
        "deletedAt",
    ] {
        assert!(
            body.get(hidden).is_none(),
            "{hidden} must not be serialized"
        );
    }
    let raw = body.to_string();
    assert!(!raw.contains("uploads/"), "no object key in {raw}");
    assert!(!raw.contains(&sha), "no checksum in {raw}");
    assert!(!raw.contains("text/csv"), "no content type in {raw}");
    assert!(
        !raw.contains(&tenant_a.to_string()),
        "no tenant id in {raw}"
    );
    for shown in [
        "id",
        "originalFilename",
        "sizeBytes",
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
/// request cannot replace the options and the table of a load in flight,
/// whether the first has been given its run yet or not (review finding B2:
/// the claim is what keeps two simultaneous requests from both launching).
#[sqlx::test(migrations = "../../migrations")]
async fn mark_ingesting_refuses_an_upload_that_is_already_loading(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-busy").await;
    add(&pool, "up-1", tenant_a, "").await;
    start(&pool, "up-1", "stock_raw", LoadMode::Replace, None).await;

    let other_options = json!({ "delimiter": ";" });
    let second = |run_id: Option<&'static str>| {
        mark_ingesting(
            &pool,
            "up-1",
            &other_options,
            "other_raw",
            LoadMode::Append,
            run_id,
        )
    };
    assert!(
        second(None).await.unwrap().is_none(),
        "a claim with no run yet is a load in flight"
    );

    attach_run(&pool, "up-1", "run-1").await.unwrap().unwrap();
    assert!(second(None).await.unwrap().is_none());
    assert!(second(Some("run-2")).await.unwrap().is_none());

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

/// A claim that never got a run (the launch was refused, or the request died
/// before `attach_run`) is settled by the same absence, `run_id = None`, and
/// by nothing else. Once a run is attached, `None` no longer settles it.
#[sqlx::test(migrations = "../../migrations")]
async fn mark_finished_with_no_run_settles_a_claim_that_never_got_one_and_nothing_else(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-finish-norun").await;
    add(&pool, "up-1", tenant_a, "").await;
    start(&pool, "up-1", "stock_raw", LoadMode::Replace, None).await;

    assert!(
        mark_finished(&pool, "up-1", Some("run-1"), None, None)
            .await
            .unwrap()
            .is_none(),
        "a run id that was never attached is not a wildcard"
    );
    let failed = mark_finished(
        &pool,
        "up-1",
        None,
        Some("The load could not be started."),
        None,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(failed.status, "failed");
    assert_eq!(
        failed.error.as_deref(),
        Some("The load could not be started.")
    );
    assert_eq!(failed.rows, None);

    // Without an error the same absence settles it as ingested.
    start(&pool, "up-1", "stock_raw", LoadMode::Replace, None).await;
    let done = mark_finished(&pool, "up-1", None, None, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.status, "ingested");

    // With a run attached, `None` is no longer what names it.
    start(&pool, "up-1", "stock_raw", LoadMode::Replace, None).await;
    attach_run(&pool, "up-1", "run-2").await.unwrap().unwrap();
    assert!(
        mark_finished(&pool, "up-1", None, None, Some(1))
            .await
            .unwrap()
            .is_none(),
        "a claim with a run is settled under that run"
    );
    assert!(
        mark_finished(&pool, "up-1", Some("run-2"), None, Some(1))
            .await
            .unwrap()
            .is_some()
    );
    Ok(())
}

/// Claim, then launch (review finding B2): the claim carries no run and
/// `attach_run` gives it the one the launch returned: once, on a loading row,
/// and without moving `updated_at`, which a fast run's end is compared to.
#[sqlx::test(migrations = "../../migrations")]
async fn attach_run_gives_a_claim_its_run_once_and_leaves_updated_at_alone(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-attach").await;
    add(&pool, "up-1", tenant_a, "").await;

    assert!(
        attach_run(&pool, "up-1", "run-1").await.unwrap().is_none(),
        "an upload nobody claimed has no load to give a run to"
    );
    assert!(
        attach_run(&pool, "up-missing", "run-1")
            .await
            .unwrap()
            .is_none()
    );

    let claimed = start(&pool, "up-1", "stock_raw", LoadMode::Replace, None).await;
    assert_eq!(claimed.status, "ingesting");
    assert!(
        claimed.run_id.is_none(),
        "the claim is made before the launch"
    );

    let attached = attach_run(&pool, "up-1", "run-1")
        .await
        .unwrap()
        .expect("a claim with no run takes one");
    assert_eq!(attached.run_id.as_deref(), Some("run-1"));
    assert_eq!(attached.status, "ingesting");
    assert_eq!(attached.bronze_table.as_deref(), Some("stock_raw"));
    assert_eq!(
        attached.updated_at, claimed.updated_at,
        "updated_at is the claim's time and stays it"
    );

    assert!(
        attach_run(&pool, "up-1", "run-2").await.unwrap().is_none(),
        "a run is attached once"
    );
    assert_eq!(
        get(&pool, "up-1").await.unwrap().unwrap().run_id.as_deref(),
        Some("run-1")
    );

    // A claim made with a run id already has one.
    add(&pool, "up-2", tenant_a, "").await;
    start(&pool, "up-2", "other_raw", LoadMode::Replace, Some("run-x")).await;
    assert!(attach_run(&pool, "up-2", "run-y").await.unwrap().is_none());

    // And a settled load has no claim left to attach to.
    mark_finished(&pool, "up-1", Some("run-1"), None, Some(3))
        .await
        .unwrap()
        .unwrap();
    assert!(attach_run(&pool, "up-1", "run-3").await.unwrap().is_none());
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

/// What the ingest route asks before it loads into a table that exists: does
/// ANY row of the SAME tenant name it. `mark_ingesting` is the only writer of
/// `bronze_table` and the route calls it after it has allowed the name, so a
/// row that names a table is a claim that was allowed, and its status does
/// not matter (review finding B1): a load still running, one that failed
/// after it had written, and one that succeeded all hold the name.
#[sqlx::test(migrations = "../../migrations")]
async fn table_claimed_by_upload_is_true_for_any_row_of_that_tenant_that_names_the_table(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-owner-a").await;
    let tenant_b = tenant(&pool, "uploads-owner-b").await;
    add(&pool, "up-1", tenant_a, "").await;

    assert!(
        !table_claimed_by_upload(&pool, tenant_a, "stock_raw")
            .await
            .unwrap(),
        "an upload that names no table claims none"
    );

    start(&pool, "up-1", "stock_raw", LoadMode::Replace, None).await;
    assert!(
        table_claimed_by_upload(&pool, tenant_a, "stock_raw")
            .await
            .unwrap(),
        "the name is claimed when the load is claimed, before the job runs"
    );

    mark_finished(
        &pool,
        "up-1",
        None,
        Some("The load into the table failed."),
        None,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(get(&pool, "up-1").await.unwrap().unwrap().status, "failed");
    assert!(
        table_claimed_by_upload(&pool, tenant_a, "stock_raw")
            .await
            .unwrap(),
        "a failed load still holds the name: the job may have written before it failed"
    );

    start(&pool, "up-1", "stock_raw", LoadMode::Replace, None).await;
    mark_finished(&pool, "up-1", None, None, Some(3))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        get(&pool, "up-1").await.unwrap().unwrap().status,
        "ingested"
    );
    assert!(
        table_claimed_by_upload(&pool, tenant_a, "stock_raw")
            .await
            .unwrap()
    );

    assert!(
        !table_claimed_by_upload(&pool, tenant_b, "stock_raw")
            .await
            .unwrap(),
        "another tenant's upload did not claim it"
    );
    assert!(
        !table_claimed_by_upload(&pool, tenant_a, "other_raw")
            .await
            .unwrap(),
        "a claim is of one name only"
    );
    Ok(())
}

/// Deleting an upload keeps its table (ADR 0014), so the row that proves an
/// upload of the tenant made it must outlive the delete. Without this a
/// deleted upload's table could never be loaded into again (finding B1).
#[sqlx::test(migrations = "../../migrations")]
async fn a_deleted_upload_still_claims_its_table_whatever_became_of_its_load(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-owner-deleted").await;
    let tenant_b = tenant(&pool, "uploads-owner-deleted-b").await;
    add(&pool, "up-1", tenant_a, "").await;
    add(&pool, "up-2", tenant_a, "").await;

    start(&pool, "up-1", "stock_raw", LoadMode::Replace, None).await;
    mark_finished(&pool, "up-1", None, None, Some(3))
        .await
        .unwrap()
        .unwrap();
    start(&pool, "up-2", "broken_raw", LoadMode::Replace, None).await;
    mark_finished(
        &pool,
        "up-2",
        None,
        Some("The load into the table failed."),
        None,
    )
    .await
    .unwrap()
    .unwrap();

    assert!(soft_delete(&pool, "up-1").await.unwrap());
    assert!(soft_delete(&pool, "up-2").await.unwrap());
    assert!(get(&pool, "up-1").await.unwrap().is_none());

    for table in ["stock_raw", "broken_raw"] {
        assert!(
            table_claimed_by_upload(&pool, tenant_a, table)
                .await
                .unwrap(),
            "{table}: the deleted row is still the claim"
        );
        assert!(
            !table_claimed_by_upload(&pool, tenant_b, table)
                .await
                .unwrap(),
            "{table}: and it is still only this tenant's"
        );
    }
    Ok(())
}

/// What a connector's ingest spec asks (plan task T8): did an upload of ANY
/// tenant load this table. Only an `ingested` row says so; deleting the
/// upload does not take it back.
#[sqlx::test(migrations = "../../migrations")]
async fn table_loaded_by_upload_is_true_only_for_an_ingested_row_of_any_tenant_deleted_or_not(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-loaded-a").await;
    let tenant_b = tenant(&pool, "uploads-loaded-b").await;
    add(&pool, "up-a", tenant_a, "").await;
    add(&pool, "up-b", tenant_b, "").await;

    assert!(
        !table_loaded_by_upload(&pool, "stock_raw").await.unwrap(),
        "an upload that names no table loaded none"
    );
    start(&pool, "up-a", "stock_raw", LoadMode::Replace, None).await;
    assert!(
        !table_loaded_by_upload(&pool, "stock_raw").await.unwrap(),
        "a load still running has not loaded it"
    );
    mark_finished(
        &pool,
        "up-a",
        None,
        Some("The load into the table failed."),
        None,
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        !table_loaded_by_upload(&pool, "stock_raw").await.unwrap(),
        "a failed load has not loaded it, as far as this record can say"
    );

    start(&pool, "up-a", "stock_raw", LoadMode::Replace, None).await;
    mark_finished(&pool, "up-a", None, None, Some(3))
        .await
        .unwrap()
        .unwrap();
    assert!(table_loaded_by_upload(&pool, "stock_raw").await.unwrap());

    // The question has no tenant: another tenant's loaded table counts the same.
    start(&pool, "up-b", "other_raw", LoadMode::Append, None).await;
    mark_finished(&pool, "up-b", None, None, None)
        .await
        .unwrap()
        .unwrap();
    assert!(table_loaded_by_upload(&pool, "other_raw").await.unwrap());
    assert!(!table_loaded_by_upload(&pool, "third_raw").await.unwrap());

    assert!(soft_delete(&pool, "up-a").await.unwrap());
    assert!(
        table_loaded_by_upload(&pool, "stock_raw").await.unwrap(),
        "deleting the upload keeps the table, and the record that it was loaded"
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

/// Deleting is soft (review finding B1): the row stays, marked, and the store
/// reports whether a live row was marked.
#[sqlx::test(migrations = "../../migrations")]
async fn soft_delete_marks_a_live_row_once_and_keeps_the_row(pool: PgPool) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-delete").await;
    add(&pool, "up-1", tenant_a, "").await;
    assert_eq!(rows_and_deleted(&pool, "up-1").await, (1, 0));

    assert!(soft_delete(&pool, "up-1").await.unwrap());
    assert_eq!(
        rows_and_deleted(&pool, "up-1").await,
        (1, 1),
        "the row is kept, with deleted_at set"
    );
    assert!(get(&pool, "up-1").await.unwrap().is_none());

    assert!(
        !soft_delete(&pool, "up-1").await.unwrap(),
        "a second delete finds no live row"
    );
    assert!(!soft_delete(&pool, "up-missing").await.unwrap());
    assert_eq!(rows_and_deleted(&pool, "up-1").await, (1, 1));
    Ok(())
}

/// A delete cannot slip in between a request that has just claimed an upload
/// and its launch: the row is refused in SQL while it is loading.
#[sqlx::test(migrations = "../../migrations")]
async fn soft_delete_refuses_an_upload_that_is_loading(pool: PgPool) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-delete-busy").await;
    add(&pool, "up-1", tenant_a, "").await;
    start(&pool, "up-1", "stock_raw", LoadMode::Replace, None).await;

    assert!(!soft_delete(&pool, "up-1").await.unwrap(), "a bare claim");
    attach_run(&pool, "up-1", "run-1").await.unwrap().unwrap();
    assert!(!soft_delete(&pool, "up-1").await.unwrap(), "a launched run");
    assert_eq!(rows_and_deleted(&pool, "up-1").await, (1, 0));
    assert_eq!(
        get(&pool, "up-1").await.unwrap().unwrap().status,
        "ingesting"
    );

    mark_finished(&pool, "up-1", Some("run-1"), None, Some(3))
        .await
        .unwrap()
        .unwrap();
    assert!(soft_delete(&pool, "up-1").await.unwrap());
    Ok(())
}

/// What "live rows only" means for the reads the console makes: a deleted
/// upload is not listed, not found by id, and in no tenant, and its
/// neighbours are unaffected.
#[sqlx::test(migrations = "../../migrations")]
async fn a_deleted_upload_is_not_listed_not_found_and_in_no_tenant(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-gone-a").await;
    add(&pool, "up-1", tenant_a, "").await;
    add(&pool, "up-2", tenant_a, "").await;
    set_created_at(&pool, "up-1", 1).await;
    assert_eq!(
        ids(&list(&pool, tenant_a, 100).await.unwrap()),
        ["up-2", "up-1"]
    );

    assert!(soft_delete(&pool, "up-1").await.unwrap());

    assert_eq!(ids(&list(&pool, tenant_a, 100).await.unwrap()), ["up-2"]);
    assert!(get(&pool, "up-1").await.unwrap().is_none());
    assert!(
        !upload_in_tenants(&pool, "up-1", &[tenant_a]).await.unwrap(),
        "the per-id routes answer 404 for a deleted upload"
    );
    assert!(upload_in_tenants(&pool, "up-2", &[tenant_a]).await.unwrap());
    Ok(())
}

/// A deleted upload cannot be loaded: its file is gone, so a claim on it
/// would launch a job that can only fail, and would name a table for nothing.
#[sqlx::test(migrations = "../../migrations")]
async fn a_deleted_upload_cannot_be_claimed_for_a_load(pool: PgPool) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-gone-claim").await;
    add(&pool, "up-1", tenant_a, "").await;
    assert!(soft_delete(&pool, "up-1").await.unwrap());

    assert!(
        mark_ingesting(
            &pool,
            "up-1",
            &options(),
            "stock_raw",
            LoadMode::Replace,
            None
        )
        .await
        .unwrap()
        .is_none()
    );
    assert!(
        !table_claimed_by_upload(&pool, tenant_a, "stock_raw")
            .await
            .unwrap(),
        "a refused claim names nothing"
    );
    let (status, table): (String, Option<String>) =
        sqlx::query_as("SELECT status, bronze_table FROM file_upload WHERE id = 'up-1'")
            .fetch_one(&pool)
            .await?;
    assert_eq!((status.as_str(), table), ("uploaded", None));
    Ok(())
}

/// A deleted upload's file is gone, so it is not an earlier copy of anything.
#[sqlx::test(migrations = "../../migrations")]
async fn a_deleted_upload_is_not_reported_as_a_duplicate(pool: PgPool) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-gone-dup").await;
    let sha = "e".repeat(64);
    add(&pool, "up-1", tenant_a, &sha).await;
    add(&pool, "up-2", tenant_a, &sha).await;
    assert_eq!(
        find_by_sha256(&pool, tenant_a, &sha, "up-2")
            .await
            .unwrap()
            .unwrap()
            .id,
        "up-1"
    );

    assert!(soft_delete(&pool, "up-1").await.unwrap());
    assert!(
        find_by_sha256(&pool, tenant_a, &sha, "up-2")
            .await
            .unwrap()
            .is_none()
    );

    add(&pool, "up-3", tenant_a, &sha).await;
    assert_eq!(
        find_by_sha256(&pool, tenant_a, &sha, "up-2")
            .await
            .unwrap()
            .unwrap()
            .id,
        "up-3",
        "a live copy is still found"
    );
    Ok(())
}

/// The writes and the loading check see live rows only too. `soft_delete`
/// refuses a loading row, so this one is made with SQL.
#[sqlx::test(migrations = "../../migrations")]
async fn a_deleted_row_is_not_loading_and_cannot_be_claimed_given_a_run_or_settled(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-gone-busy").await;
    add(&pool, "up-1", tenant_a, "").await;
    start(&pool, "up-1", "stock_raw", LoadMode::Replace, None).await;
    assert!(
        table_being_loaded(&pool, "stock_raw", "up-other")
            .await
            .unwrap()
    );

    force_deleted(&pool, "up-1").await;

    assert!(
        !table_being_loaded(&pool, "stock_raw", "up-other")
            .await
            .unwrap(),
        "a deleted row holds no name"
    );
    assert!(
        mark_ingesting(
            &pool,
            "up-1",
            &options(),
            "other_raw",
            LoadMode::Append,
            None
        )
        .await
        .unwrap()
        .is_none()
    );
    assert!(attach_run(&pool, "up-1", "run-1").await.unwrap().is_none());
    assert!(
        mark_finished(&pool, "up-1", None, None, Some(1))
            .await
            .unwrap()
            .is_none()
    );
    let (status, table): (String, Option<String>) =
        sqlx::query_as("SELECT status, bronze_table FROM file_upload WHERE id = 'up-1'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(
        (status.as_str(), table.as_deref()),
        ("ingesting", Some("stock_raw")),
        "none of those writes touched the row"
    );
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
    for added in ["tenant_id", "load_mode", "row_count", "deleted_at"] {
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
