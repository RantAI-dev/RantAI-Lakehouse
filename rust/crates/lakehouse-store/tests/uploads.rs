//! Integration tests for `lakehouse_store::uploads` and the `file_upload`
//! table (`0057_upload.sql`, `0058_upload_tenant_mode.sql`) against a real
//! Postgres.
//!
//! # Postgres backing
//!
//! These are `#[sqlx::test(migrations = "../../migrations")]` tests: each
//! one gets a freshly migrated, isolated database, which is also how
//! `0058` is proven to apply on a fresh one. The Postgres *server* itself
//! is started once per test binary by the `lakehouse-test-support`
//! dev-dependency (see `tests/connectors.rs`). The two migration tests
//! below start from an EMPTY database and apply `0057` first, then the
//! rest, to prove `0058` applies on a database that already has `0057`.
//!
//! `any_connector_targets` (the other store function T3 adds) is tested in
//! `tests/connectors.rs`, beside the connector helpers it needs.
//!
//! T5a of the plan (review findings B1, B2 and B3) amended the store after
//! these tests were first written: a load is claimed before it is launched,
//! and `content_type` and `sha256` are not on the wire. T6a (finding B4)
//! amended it again: who owns a table name is a record of its own,
//! `upload_table_claim`, keyed by the name, and deleting an upload is a real
//! delete (T5a had made it soft to keep the row as that record). The tests for
//! each are named after what they pin.

#![allow(clippy::unwrap_used, clippy::expect_used)]

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap actually runs for this test binary.
use lakehouse_test_support as _;

use std::borrow::Cow;

use lakehouse_store::StoreError;
use lakehouse_store::identity::{CreateTenantInput, create_tenant};
use lakehouse_store::uploads::{
    LoadMode, NewUpload, TableClaim, Upload, attach_run, claim_table, delete, find_by_sha256, get,
    insert, list, mark_finished, mark_ingesting, table_being_loaded, table_claim, table_claimed,
    upload_in_tenants,
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

/// How many rows `file_upload` holds under this id, read around the store's
/// own functions.
async fn rows_with_id(pool: &PgPool, id: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM file_upload WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Claim `table` for `tenant_id` the way the ingest route does before it marks
/// an upload as loading; the claim must be allowed.
async fn claim(pool: &PgPool, tenant_id: Uuid, table: &str, upload_id: &str) {
    assert!(
        claim_table(pool, tenant_id, table, upload_id)
            .await
            .unwrap(),
        "{table} could not be claimed for {tenant_id}"
    );
}

/// `(tenant, upload that asked first)` of the claim on `table`, read around
/// the store's own functions, or `None` when nothing is claimed.
async fn claim_row(pool: &PgPool, table: &str) -> Option<(Option<Uuid>, String)> {
    sqlx::query_as("SELECT tenant_id, upload_id FROM upload_table_claim WHERE bronze_table = $1")
        .bind(table)
        .fetch_optional(pool)
        .await
        .unwrap()
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

    for hidden in ["storageKey", "tenantId", "tenant", "contentType", "sha256"] {
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

// ── Who owns a table name (T6a, review finding B4) ─────────────────────────

/// The first claim on a name is made, the same tenant's is held again, and
/// another tenant's is refused. What `claim_table` answers is whether the claim
/// is the asking tenant's afterwards.
#[sqlx::test(migrations = "../../migrations")]
async fn claim_table_is_true_for_the_first_asker_and_the_holder_again_and_false_for_anyone_else(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-claim-a").await;
    let tenant_b = tenant(&pool, "uploads-claim-b").await;

    assert!(
        claim_table(&pool, tenant_a, "stock_raw", "up-a1")
            .await
            .unwrap(),
        "a name nobody holds is claimed"
    );
    assert!(
        claim_table(&pool, tenant_a, "stock_raw", "up-a2")
            .await
            .unwrap(),
        "held already: still this tenant's, whichever of its uploads asks"
    );
    assert!(
        !claim_table(&pool, tenant_b, "stock_raw", "up-b1")
            .await
            .unwrap(),
        "another tenant holds it"
    );
    assert!(
        !claim_table(&pool, tenant_b, "stock_raw", "up-b2")
            .await
            .unwrap(),
        "and asking again does not change that"
    );

    // The record is the first asker's, and nothing else was made.
    assert_eq!(
        claim_row(&pool, "stock_raw").await,
        Some((Some(tenant_a), "up-a1".to_owned())),
        "the claim keeps the upload that asked first and the tenant that held it"
    );
    let all: i64 = sqlx::query_scalar("SELECT count(*) FROM upload_table_claim")
        .fetch_one(&pool)
        .await?;
    assert_eq!(all, 1, "the refused asks made no claim");

    // A claim is of one name only.
    assert!(
        claim_table(&pool, tenant_b, "other_raw", "up-b1")
            .await
            .unwrap()
    );
    assert!(
        !claim_table(&pool, tenant_a, "other_raw", "up-a1")
            .await
            .unwrap()
    );
    Ok(())
}

/// Review finding B4: two tenants asking for one new name at the same moment
/// cannot both win. `claim_table` is one statement and the primary key is the
/// arbiter, so in every round, each with a name of its own, exactly one of the
/// two gets `true`. A check followed by an insert would let both through in
/// some round, or fail one of them with a unique violation, which `unwrap`
/// turns into a failure of this test.
#[sqlx::test(migrations = "../../migrations")]
async fn two_tenants_claiming_one_new_name_at_the_same_moment_one_wins(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-race-a").await;
    let tenant_b = tenant(&pool, "uploads-race-b").await;

    for round in 0..25 {
        let table = format!("race_{round}");
        let (a, b) = tokio::join!(
            claim_table(&pool, tenant_a, &table, "up-a"),
            claim_table(&pool, tenant_b, &table, "up-b"),
        );
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_ne!(a, b, "round {round}: exactly one claim wins (a={a}, b={b})");

        let (winner, loser) = if a {
            (tenant_a, tenant_b)
        } else {
            (tenant_b, tenant_a)
        };
        assert_eq!(
            table_claim(&pool, winner, &table).await.unwrap(),
            TableClaim::Ours,
            "round {round}"
        );
        assert_eq!(
            table_claim(&pool, loser, &table).await.unwrap(),
            TableClaim::Theirs,
            "round {round}"
        );
    }

    // The same tenant asking twice at once is no conflict: both are told it is
    // theirs, and the first asker's upload is the one recorded.
    let (first, second) = tokio::join!(
        claim_table(&pool, tenant_a, "twice_raw", "up-1"),
        claim_table(&pool, tenant_a, "twice_raw", "up-2"),
    );
    assert!(first.unwrap() && second.unwrap());
    let recorded = claim_row(&pool, "twice_raw").await.unwrap();
    assert_eq!(recorded.0, Some(tenant_a));
    assert!(["up-1", "up-2"].contains(&recorded.1.as_str()));
    Ok(())
}

/// `table_claim` is the read the route makes before it asks the database to
/// decide: nobody's, the asking tenant's, or someone else's. Reading claims
/// nothing.
#[sqlx::test(migrations = "../../migrations")]
async fn table_claim_answers_unclaimed_ours_or_theirs_and_claims_nothing(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-read-a").await;
    let tenant_b = tenant(&pool, "uploads-read-b").await;

    for asking in [tenant_a, tenant_b] {
        assert_eq!(
            table_claim(&pool, asking, "stock_raw").await.unwrap(),
            TableClaim::Unclaimed
        );
    }
    assert!(
        claim_row(&pool, "stock_raw").await.is_none(),
        "asking made no claim"
    );

    claim(&pool, tenant_a, "stock_raw", "up-a1").await;
    assert_eq!(
        table_claim(&pool, tenant_a, "stock_raw").await.unwrap(),
        TableClaim::Ours
    );
    assert_eq!(
        table_claim(&pool, tenant_b, "stock_raw").await.unwrap(),
        TableClaim::Theirs
    );
    assert_eq!(
        table_claim(&pool, tenant_a, "other_raw").await.unwrap(),
        TableClaim::Unclaimed,
        "a claim is of one name only"
    );
    Ok(())
}

/// What a connector's ingest spec asks (plan task T8): does ANY tenant hold
/// the name. No tenant is asking, so it is not `table_claim`.
#[sqlx::test(migrations = "../../migrations")]
async fn table_claimed_is_true_for_a_claim_of_any_tenant_and_false_for_a_name_nobody_asked_for(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-any-a").await;
    let tenant_b = tenant(&pool, "uploads-any-b").await;

    assert!(!table_claimed(&pool, "stock_raw").await.unwrap());
    claim(&pool, tenant_a, "stock_raw", "up-a1").await;
    claim(&pool, tenant_b, "other_raw", "up-b1").await;
    assert!(table_claimed(&pool, "stock_raw").await.unwrap());
    assert!(
        table_claimed(&pool, "other_raw").await.unwrap(),
        "the question has no tenant: another tenant's claim counts the same"
    );
    assert!(!table_claimed(&pool, "third_raw").await.unwrap());
    Ok(())
}

/// A claim whose tenant was deleted belongs to nobody (`ON DELETE SET NULL`,
/// the shape of `file_upload.tenant_id`), and nobody may take it over: no
/// tenant is told it is theirs, no tenant's claim succeeds, and it still counts
/// as claimed. The name stays reserved for good (fail closed).
#[sqlx::test(migrations = "../../migrations")]
async fn a_claim_whose_tenant_is_gone_belongs_to_nobody_and_stays_reserved(
    pool: PgPool,
) -> sqlx::Result<()> {
    let gone = tenant(&pool, "uploads-gone").await;
    let tenant_b = tenant(&pool, "uploads-gone-b").await;
    let tenant_c = tenant(&pool, "uploads-gone-c").await;
    claim(&pool, gone, "stock_raw", "up-gone").await;

    sqlx::query("DELETE FROM tenant WHERE id = $1")
        .bind(gone)
        .execute(&pool)
        .await?;

    assert_eq!(
        claim_row(&pool, "stock_raw").await,
        Some((None, "up-gone".to_owned())),
        "the claim stays, with no tenant"
    );
    for asking in [tenant_b, tenant_c] {
        assert!(
            !claim_table(&pool, asking, "stock_raw", "up-x")
                .await
                .unwrap(),
            "nobody takes over a claim that has no tenant"
        );
        assert_eq!(
            table_claim(&pool, asking, "stock_raw").await.unwrap(),
            TableClaim::Theirs,
            "it is no asking tenant's"
        );
    }
    assert_eq!(
        table_claim(&pool, gone, "stock_raw").await.unwrap(),
        TableClaim::Theirs,
        "not even the id of the tenant that is gone"
    );
    assert!(table_claimed(&pool, "stock_raw").await.unwrap());
    assert_eq!(
        claim_row(&pool, "stock_raw").await,
        Some((None, "up-gone".to_owned())),
        "and it is unchanged by the asks"
    );
    Ok(())
}

/// A claim is never released (review finding B4): not when the load it was
/// made for fails (the job may have written before it failed), not when the
/// upload is loaded into another table, and not when the upload is deleted.
#[sqlx::test(migrations = "../../migrations")]
async fn a_claim_is_never_released_by_a_failed_load_another_load_or_a_delete(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-keep-a").await;
    add(&pool, "up-1", tenant_a, "").await;

    claim(&pool, tenant_a, "stock_raw", "up-1").await;
    start(&pool, "up-1", "stock_raw", LoadMode::Replace, Some("run-1")).await;
    let failed = mark_finished(
        &pool,
        "up-1",
        Some("run-1"),
        Some("The load into the table failed."),
        None,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(failed.status, "failed");
    assert_eq!(
        table_claim(&pool, tenant_a, "stock_raw").await.unwrap(),
        TableClaim::Ours,
        "a failed load still holds the name"
    );

    claim(&pool, tenant_a, "other_raw", "up-1").await;
    start(&pool, "up-1", "other_raw", LoadMode::Replace, Some("run-2")).await;
    mark_finished(&pool, "up-1", Some("run-2"), None, Some(3))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        table_claim(&pool, tenant_a, "stock_raw").await.unwrap(),
        TableClaim::Ours,
        "loading the upload into another table does not release the first"
    );

    assert!(delete(&pool, "up-1").await.unwrap());
    for table in ["stock_raw", "other_raw"] {
        assert_eq!(
            table_claim(&pool, tenant_a, table).await.unwrap(),
            TableClaim::Ours,
            "{table}: deleting the upload does not release it"
        );
        assert!(table_claimed(&pool, table).await.unwrap(), "{table}");
        assert_eq!(
            claim_row(&pool, table).await,
            Some((Some(tenant_a), "up-1".to_owned())),
            "{table}: the record still names the upload that asked"
        );
    }
    Ok(())
}

/// The failure the review found (finding B4): tenant B's load into `t` fails
/// before it writes; tenant A then asks for `t`; B retries. With ownership read
/// from upload rows, A took `t` and B's retry replaced A's rows. With the claim
/// recorded, B's failed claim keeps A out and B's retry still passes.
#[sqlx::test(migrations = "../../migrations")]
async fn a_failed_claim_keeps_another_tenant_out_and_lets_its_own_tenant_retry(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-retry-a").await;
    let tenant_b = tenant(&pool, "uploads-retry-b").await;
    add(&pool, "up-a1", tenant_a, "").await;
    add(&pool, "up-b1", tenant_b, "").await;

    claim(&pool, tenant_b, "t", "up-b1").await;
    start(&pool, "up-b1", "t", LoadMode::Replace, None).await;
    mark_finished(
        &pool,
        "up-b1",
        None,
        Some("The load could not be started."),
        None,
    )
    .await
    .unwrap()
    .unwrap();

    assert!(
        !claim_table(&pool, tenant_a, "t", "up-a1").await.unwrap(),
        "B's claim stands although B's load failed"
    );
    assert_eq!(
        table_claim(&pool, tenant_a, "t").await.unwrap(),
        TableClaim::Theirs
    );

    assert!(
        claim_table(&pool, tenant_b, "t", "up-b1").await.unwrap(),
        "B's retry is B's own claim"
    );
    assert_eq!(
        table_claim(&pool, tenant_b, "t").await.unwrap(),
        TableClaim::Ours
    );
    start(&pool, "up-b1", "t", LoadMode::Replace, None).await;
    Ok(())
}

/// The other failure (finding B4): an upload that loaded `x` and is then
/// loaded into `y` names only `y`, so nothing in its row says an upload made
/// `x`. The claim on `x` does: another upload of the tenant may load into it,
/// another tenant may not, and `table_claimed` says a connector may not either.
#[sqlx::test(migrations = "../../migrations")]
async fn an_upload_loaded_into_one_table_and_then_another_leaves_the_first_claimed(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-two-tables-a").await;
    let tenant_b = tenant(&pool, "uploads-two-tables-b").await;
    add(&pool, "up-1", tenant_a, "").await;
    add(&pool, "up-2", tenant_a, "").await;

    claim(&pool, tenant_a, "x_raw", "up-1").await;
    start(&pool, "up-1", "x_raw", LoadMode::Replace, Some("run-1")).await;
    mark_finished(&pool, "up-1", Some("run-1"), None, Some(5))
        .await
        .unwrap()
        .unwrap();
    claim(&pool, tenant_a, "y_raw", "up-1").await;
    start(&pool, "up-1", "y_raw", LoadMode::Replace, Some("run-2")).await;

    let row = get(&pool, "up-1").await.unwrap().unwrap();
    assert_eq!(
        row.bronze_table.as_deref(),
        Some("y_raw"),
        "the row names only its last load"
    );
    for table in ["x_raw", "y_raw"] {
        assert_eq!(
            table_claim(&pool, tenant_a, table).await.unwrap(),
            TableClaim::Ours,
            "{table}"
        );
        assert!(table_claimed(&pool, table).await.unwrap(), "{table}");
    }

    // Another upload of the same tenant may load into the first table ...
    claim(&pool, tenant_a, "x_raw", "up-2").await;
    start(&pool, "up-2", "x_raw", LoadMode::Append, Some("run-3")).await;
    assert_eq!(
        claim_row(&pool, "x_raw").await,
        Some((Some(tenant_a), "up-1".to_owned())),
        "the claim is still the one the first upload made"
    );
    // ... another tenant may not.
    assert!(
        !claim_table(&pool, tenant_b, "x_raw", "up-b1")
            .await
            .unwrap()
    );
    assert_eq!(
        table_claim(&pool, tenant_b, "x_raw").await.unwrap(),
        TableClaim::Theirs
    );
    Ok(())
}

/// After a delete the claim remains, and a new upload of the tenant loads into
/// the table (finding B4; T5a's soft delete existed to keep this true).
#[sqlx::test(migrations = "../../migrations")]
async fn after_a_delete_the_claim_remains_and_a_new_upload_of_the_tenant_loads_into_the_table(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-after-delete-a").await;
    let tenant_b = tenant(&pool, "uploads-after-delete-b").await;
    add(&pool, "up-1", tenant_a, "").await;
    claim(&pool, tenant_a, "stock_raw", "up-1").await;
    start(&pool, "up-1", "stock_raw", LoadMode::Replace, Some("run-1")).await;
    mark_finished(&pool, "up-1", Some("run-1"), None, Some(3))
        .await
        .unwrap()
        .unwrap();

    assert!(delete(&pool, "up-1").await.unwrap());
    assert_eq!(rows_with_id(&pool, "up-1").await, 0, "the row is gone");

    add(&pool, "up-2", tenant_a, "").await;
    assert_eq!(
        table_claim(&pool, tenant_a, "stock_raw").await.unwrap(),
        TableClaim::Ours
    );
    claim(&pool, tenant_a, "stock_raw", "up-2").await;
    let again = start(&pool, "up-2", "stock_raw", LoadMode::Append, None).await;
    assert_eq!(again.bronze_table.as_deref(), Some("stock_raw"));

    assert!(
        !claim_table(&pool, tenant_b, "stock_raw", "up-b1")
            .await
            .unwrap(),
        "and it is still closed to another tenant"
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

/// Deleting is a real delete again (T6a; T5a had made it soft): the row goes,
/// the store reports whether one was deleted, and what the upload became is not
/// its to take back.
#[sqlx::test(migrations = "../../migrations")]
async fn delete_removes_the_row_once_and_leaves_its_table_claim(pool: PgPool) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-delete").await;
    add(&pool, "up-1", tenant_a, "").await;
    add(&pool, "up-2", tenant_a, "").await;
    claim(&pool, tenant_a, "stock_raw", "up-1").await;
    start(&pool, "up-1", "stock_raw", LoadMode::Replace, Some("run-1")).await;
    mark_finished(&pool, "up-1", Some("run-1"), None, Some(3))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rows_with_id(&pool, "up-1").await, 1);

    assert!(delete(&pool, "up-1").await.unwrap());
    assert_eq!(rows_with_id(&pool, "up-1").await, 0, "the row is gone");
    assert!(get(&pool, "up-1").await.unwrap().is_none());
    assert_eq!(
        rows_with_id(&pool, "up-2").await,
        1,
        "another upload is untouched"
    );

    assert!(
        !delete(&pool, "up-1").await.unwrap(),
        "a second delete finds nothing"
    );
    assert!(!delete(&pool, "up-missing").await.unwrap());
    assert_eq!(
        table_claim(&pool, tenant_a, "stock_raw").await.unwrap(),
        TableClaim::Ours,
        "the claim on the table the upload loaded is not the upload's to take back"
    );
    Ok(())
}

/// A delete cannot slip in between a request that has just claimed an upload
/// and its launch: the row is refused in SQL while it is loading.
#[sqlx::test(migrations = "../../migrations")]
async fn delete_refuses_an_upload_that_is_loading(pool: PgPool) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-delete-busy").await;
    add(&pool, "up-1", tenant_a, "").await;
    start(&pool, "up-1", "stock_raw", LoadMode::Replace, None).await;

    assert!(!delete(&pool, "up-1").await.unwrap(), "a bare claim");
    attach_run(&pool, "up-1", "run-1").await.unwrap().unwrap();
    assert!(!delete(&pool, "up-1").await.unwrap(), "a launched run");
    assert_eq!(rows_with_id(&pool, "up-1").await, 1);
    assert_eq!(
        get(&pool, "up-1").await.unwrap().unwrap().status,
        "ingesting"
    );

    mark_finished(&pool, "up-1", Some("run-1"), None, Some(3))
        .await
        .unwrap()
        .unwrap();
    assert!(delete(&pool, "up-1").await.unwrap());
    Ok(())
}

/// What a delete means for the reads the console makes: a deleted upload is
/// not listed, not found by id, in no tenant, not a duplicate and not loading,
/// its neighbours are unaffected, and nothing can be claimed, given a run or
/// settled on it any more.
#[sqlx::test(migrations = "../../migrations")]
async fn a_deleted_upload_is_not_listed_not_found_not_a_duplicate_and_cannot_be_loaded(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-gone-a").await;
    let sha = "e".repeat(64);
    add(&pool, "up-1", tenant_a, &sha).await;
    add(&pool, "up-2", tenant_a, &sha).await;
    set_created_at(&pool, "up-1", 1).await;
    assert_eq!(
        ids(&list(&pool, tenant_a, 100).await.unwrap()),
        ["up-2", "up-1"]
    );
    assert_eq!(
        find_by_sha256(&pool, tenant_a, &sha, "up-2")
            .await
            .unwrap()
            .unwrap()
            .id,
        "up-1"
    );

    assert!(delete(&pool, "up-1").await.unwrap());

    assert_eq!(ids(&list(&pool, tenant_a, 100).await.unwrap()), ["up-2"]);
    assert!(get(&pool, "up-1").await.unwrap().is_none());
    assert!(
        !upload_in_tenants(&pool, "up-1", &[tenant_a]).await.unwrap(),
        "the per-id routes answer 404 for a deleted upload"
    );
    assert!(upload_in_tenants(&pool, "up-2", &[tenant_a]).await.unwrap());
    assert!(
        find_by_sha256(&pool, tenant_a, &sha, "up-2")
            .await
            .unwrap()
            .is_none(),
        "a deleted upload's file is gone, so it is not an earlier copy of anything"
    );
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
    assert!(attach_run(&pool, "up-1", "run-1").await.unwrap().is_none());
    assert!(
        mark_finished(&pool, "up-1", None, None, Some(1))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        !table_being_loaded(&pool, "stock_raw", "up-other")
            .await
            .unwrap(),
        "a deleted upload holds no name"
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

/// The first half of "does 0058 apply on a database that has 0057": `0057`
/// applied, no rows, then everything after it. (The other half, a fresh
/// database, is every `#[sqlx::test(migrations = ...)]` in this crate.)
#[sqlx::test(migrations = false)]
async fn migration_0058_applies_on_a_database_that_has_0057_and_no_rows(
    pool: PgPool,
) -> sqlx::Result<()> {
    migrations_up_to(57).run(&pool).await.unwrap();
    let before = file_upload_columns(&pool).await;
    assert!(before.contains(&"tenant".to_owned()));
    assert!(!before.contains(&"tenant_id".to_owned()));

    MIGRATOR.run(&pool).await.unwrap();

    // Exactly what `0058` adds to `file_upload` and exactly what it drops. T5a
    // added a column here that T6a took out again (review finding B4); asking
    // for the exact sets is what says it is not there.
    let after = file_upload_columns(&pool).await;
    let added: Vec<&str> = after
        .iter()
        .filter(|column| !before.contains(column))
        .map(String::as_str)
        .collect();
    let dropped: Vec<&str> = before
        .iter()
        .filter(|column| !after.contains(column))
        .map(String::as_str)
        .collect();
    assert_eq!(added, ["tenant_id", "load_mode", "row_count"]);
    assert_eq!(dropped, ["tenant"], "the free-text tenant column is gone");

    // And the table of claims, empty.
    assert_eq!(
        table_columns(&pool, "upload_table_claim").await,
        ["bronze_table", "tenant_id", "upload_id", "claimed_at"]
    );
    let claims: i64 = sqlx::query_scalar("SELECT count(*) FROM upload_table_claim")
        .fetch_one(&pool)
        .await?;
    assert_eq!(claims, 0);
    Ok(())
}

/// The claim table's own rules, below the store function that uses them
/// (review finding B4): one row per table name, decided by the primary key; an
/// upload id is always recorded; a tenant that does not exist cannot hold a
/// claim, and `claim_table` says so as the foreign-key violation `insert`
/// answers too.
#[sqlx::test(migrations = "../../migrations")]
async fn the_claim_table_has_one_row_per_table_name_and_needs_an_upload_and_a_real_tenant(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = tenant(&pool, "uploads-claim-shape").await;
    sqlx::query("INSERT INTO upload_table_claim (bronze_table, tenant_id, upload_id) VALUES ('t', $1, 'up-1')")
        .bind(tenant_a)
        .execute(&pool)
        .await?;

    let twice = sqlx::query(
        "INSERT INTO upload_table_claim (bronze_table, tenant_id, upload_id) VALUES ('t', $1, 'up-2')",
    )
    .bind(tenant_a)
    .execute(&pool)
    .await
    .unwrap_err();
    assert!(
        twice
            .as_database_error()
            .is_some_and(sqlx::error::DatabaseError::is_unique_violation),
        "the table name is the primary key, got {twice:?}"
    );

    let no_upload =
        sqlx::query("INSERT INTO upload_table_claim (bronze_table, tenant_id) VALUES ('u', $1)")
            .bind(tenant_a)
            .execute(&pool)
            .await
            .unwrap_err();
    assert_eq!(
        no_upload
            .as_database_error()
            .map(sqlx::error::DatabaseError::kind),
        Some(sqlx::error::ErrorKind::NotNullViolation),
        "{no_upload:?}"
    );

    let stranger = claim_table(&pool, Uuid::new_v4(), "v_raw", "up-3")
        .await
        .unwrap_err();
    assert!(
        matches!(stranger, StoreError::ForeignKeyViolation),
        "{stranger:?}"
    );
    assert!(claim_row(&pool, "v_raw").await.is_none());
    // A name that is already held answers false without a row of the asking
    // tenant ever being written, so there is no foreign key to violate: this is
    // what `claim_table`'s `# Errors` says.
    assert!(
        !claim_table(&pool, Uuid::new_v4(), "t", "up-4")
            .await
            .unwrap()
    );
    assert_eq!(
        claim_row(&pool, "t").await,
        Some((Some(tenant_a), "up-1".to_owned()))
    );

    let recent: bool = sqlx::query_scalar(
        "SELECT claimed_at <= now() AND claimed_at > now() - interval '1 minute' \
         FROM upload_table_claim WHERE bronze_table = 't'",
    )
    .fetch_one(&pool)
    .await?;
    assert!(recent, "claimed_at defaults to the time of the claim");
    Ok(())
}

/// What `0058` does to a row `0057` could in principle have held: it stays,
/// with no tenant, and every tenant-scoped read ignores it. Nothing wrote
/// such a row (the header of `0058` says why); this pins what would happen.
#[sqlx::test(migrations = false)]
async fn migration_0058_keeps_a_row_written_before_it_but_hides_it_from_every_tenant(
    pool: PgPool,
) -> sqlx::Result<()> {
    migrations_up_to(57).run(&pool).await.unwrap();
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
    table_columns(pool, "file_upload").await
}

async fn table_columns(pool: &PgPool, table: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT column_name::text FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name = $1 \
         ORDER BY ordinal_position",
    )
    .bind(table)
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
