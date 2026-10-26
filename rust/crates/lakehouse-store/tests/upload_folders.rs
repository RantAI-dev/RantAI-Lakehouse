//! Integration tests for `lakehouse_store::upload_folders`, the folder columns
//! on `file_upload` and `upload_table_claim`, and `0112_upload_folders.sql`
//! (`DATA-35`, plan `docs/superpowers/plans/2026-10-26-data-35-upload-folders.md`,
//! T1) against a real Postgres.
//!
//! `#[sqlx::test(migrations = "../../migrations")]` gives each test a freshly
//! migrated database; the server is started once per binary by
//! `lakehouse-test-support`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap actually runs for this test binary.
use lakehouse_test_support as _;

use std::borrow::Cow;

use lakehouse_store::identity::{CreateTenantInput, create_tenant};
use lakehouse_store::upload_folders::{
    FolderError, FolderPatch, MAX_DEPTH, ParentChange, claim_folder, create, delete,
    folder_in_tenant, list, move_uploads, update,
};
use lakehouse_store::uploads::{
    FolderFilter, ListFilter, LoadClaim, LoadLimits, LoadMode, MarkOutcome, NewUpload, Upload,
    claim_table, get, insert, list_filtered, mark_group_within_limits,
};
use serde_json::json;
use sqlx::PgPool;
use sqlx::migrate::Migrator;
use uuid::Uuid;

/// The embedded migration set, to apply in two steps in the migration test.
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

async fn folder(pool: &PgPool, tenant_id: Uuid, id: &str, parent: Option<&str>, name: &str) {
    create(pool, tenant_id, id, parent, name, "Tester")
        .await
        .unwrap();
}

async fn add(pool: &PgPool, id: &str, tenant_id: Uuid, filename: &str) -> Upload {
    let key = format!("uploads/{tenant_id}/{id}.csv");
    insert(
        pool,
        &NewUpload {
            id,
            original_filename: filename,
            storage_key: &key,
            content_type: "text/csv",
            size_bytes: 10,
            sha256: id,
            uploaded_by: "Tester",
            tenant_id,
        },
    )
    .await
    .unwrap()
}

async fn show(pool: &PgPool, t: Uuid, folder: FolderFilter<'_>, text: Option<&str>) -> Vec<String> {
    let rows = list_filtered(pool, t, 100, &ListFilter { folder, text })
        .await
        .unwrap();
    rows.into_iter().map(|u| u.id).collect()
}

/// Decision 3: three levels are allowed and a fourth is refused, on create.
#[sqlx::test(migrations = "../../migrations")]
async fn a_fourth_level_is_refused_on_create(pool: PgPool) -> sqlx::Result<()> {
    let t = tenant(&pool, "folders-depth").await;
    assert_eq!(MAX_DEPTH, 3);
    folder(&pool, t, "uf-1", None, "A").await;
    folder(&pool, t, "uf-2", Some("uf-1"), "B").await;
    folder(&pool, t, "uf-3", Some("uf-2"), "C").await;

    let refused = create(&pool, t, "uf-4", Some("uf-3"), "D", "Tester").await;
    assert!(matches!(refused, Err(FolderError::TooDeep)));
    assert_eq!(list(&pool, t).await.unwrap().len(), 3, "nothing was made");
    Ok(())
}

/// A move counts the moved folder's own sub-folders: a folder with a child
/// cannot go under a folder that is already two deep.
#[sqlx::test(migrations = "../../migrations")]
async fn a_move_counts_the_subtree_of_the_moved_folder(pool: PgPool) -> sqlx::Result<()> {
    let t = tenant(&pool, "folders-move-depth").await;
    folder(&pool, t, "uf-a", None, "A").await;
    folder(&pool, t, "uf-b", Some("uf-a"), "B").await;
    folder(&pool, t, "uf-x", None, "X").await;
    folder(&pool, t, "uf-y", Some("uf-x"), "Y").await;

    // X (with Y below it) under B: A > B > X > Y is four levels.
    let refused = update(
        &pool,
        t,
        "uf-x",
        FolderPatch {
            parent: ParentChange::Under("uf-b"),
            ..FolderPatch::default()
        },
    )
    .await;
    assert!(matches!(refused, Err(FolderError::TooDeep)));

    // Y alone under B: three levels, allowed.
    let moved = update(
        &pool,
        t,
        "uf-y",
        FolderPatch {
            parent: ParentChange::Under("uf-b"),
            ..FolderPatch::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(moved.parent_id.as_deref(), Some("uf-b"));
    Ok(())
}

/// A folder cannot be moved into itself or into one of its own folders.
#[sqlx::test(migrations = "../../migrations")]
async fn a_folder_cannot_be_moved_into_itself_or_its_descendant(pool: PgPool) -> sqlx::Result<()> {
    let t = tenant(&pool, "folders-cycle").await;
    folder(&pool, t, "uf-a", None, "A").await;
    folder(&pool, t, "uf-b", Some("uf-a"), "B").await;

    for target in ["uf-a", "uf-b"] {
        let refused = update(
            &pool,
            t,
            "uf-a",
            FolderPatch {
                parent: ParentChange::Under(target),
                ..FolderPatch::default()
            },
        )
        .await;
        assert!(matches!(refused, Err(FolderError::Cycle)), "into {target}");
    }
    Ok(())
}

/// Decision 6: names are unique among siblings, compared without case, on
/// create, rename and move; the same name under another parent is fine.
#[sqlx::test(migrations = "../../migrations")]
async fn sibling_names_are_unique_without_regard_to_case(pool: PgPool) -> sqlx::Result<()> {
    let t = tenant(&pool, "folders-names").await;
    folder(&pool, t, "uf-1", None, "Sales").await;
    folder(&pool, t, "uf-2", None, "Ops").await;
    folder(&pool, t, "uf-3", Some("uf-2"), "Archive").await;

    let top = create(&pool, t, "uf-4", None, "  sALES ", "Tester").await;
    assert!(matches!(top, Err(FolderError::NameTaken)), "top level");
    // Same name, other parent: allowed.
    folder(&pool, t, "uf-5", Some("uf-1"), "Archive").await;

    let rename = update(
        &pool,
        t,
        "uf-2",
        FolderPatch {
            name: Some("SALES"),
            ..FolderPatch::default()
        },
    )
    .await;
    assert!(matches!(rename, Err(FolderError::NameTaken)), "rename");

    // Moving "Archive" of Ops under Sales collides with Sales' own "Archive".
    let moved = update(
        &pool,
        t,
        "uf-3",
        FolderPatch {
            parent: ParentChange::Under("uf-1"),
            ..FolderPatch::default()
        },
    )
    .await;
    assert!(matches!(moved, Err(FolderError::NameTaken)), "move");

    // Renaming a folder to its own name in another case is not a collision.
    let same = update(
        &pool,
        t,
        "uf-1",
        FolderPatch {
            name: Some("sales"),
            ..FolderPatch::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(same.name, "sales");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_name_is_one_to_a_hundred_characters_after_trim(pool: PgPool) -> sqlx::Result<()> {
    let t = tenant(&pool, "folders-length").await;
    for bad in ["", "   ", &"x".repeat(101)] {
        let refused = create(&pool, t, "uf-bad", None, bad, "Tester").await;
        assert!(matches!(refused, Err(FolderError::NameInvalid)), "{bad:?}");
    }
    let made = create(
        &pool,
        t,
        "uf-ok",
        None,
        &format!(" {} ", "é".repeat(100)),
        "Tester",
    )
    .await
    .unwrap();
    assert_eq!(
        made.name.chars().count(),
        100,
        "trimmed, counted in characters"
    );
    Ok(())
}

/// Decision 8: a folder holding a file, or a folder, cannot be deleted; an empty
/// one can, and its id is then unknown.
#[sqlx::test(migrations = "../../migrations")]
async fn delete_is_refused_with_a_file_and_with_a_sub_folder(pool: PgPool) -> sqlx::Result<()> {
    let t = tenant(&pool, "folders-delete").await;
    folder(&pool, t, "uf-a", None, "A").await;
    folder(&pool, t, "uf-b", Some("uf-a"), "B").await;
    add(&pool, "up-1", t, "one.csv").await;
    move_uploads(&pool, t, &["up-1".to_owned()], Some("uf-b"))
        .await
        .unwrap();

    assert!(matches!(
        delete(&pool, t, "uf-a").await,
        Err(FolderError::NotEmpty)
    ));
    assert!(matches!(
        delete(&pool, t, "uf-b").await,
        Err(FolderError::NotEmpty)
    ));

    let listed = list(&pool, t).await.unwrap();
    let counts: Vec<(&str, i64, i64)> = listed
        .iter()
        .map(|f| (f.id.as_str(), f.file_count, f.child_count))
        .collect();
    assert_eq!(counts, [("uf-a", 0, 1), ("uf-b", 1, 0)]);

    move_uploads(&pool, t, &["up-1".to_owned()], None)
        .await
        .unwrap();
    delete(&pool, t, "uf-b").await.unwrap();
    delete(&pool, t, "uf-a").await.unwrap();
    assert!(list(&pool, t).await.unwrap().is_empty());
    assert!(matches!(
        delete(&pool, t, "uf-a").await,
        Err(FolderError::NotFound)
    ));
    Ok(())
}

/// Decision 5, `SEC-16`: another tenant's folder is not found by any function,
/// and its files cannot be moved into it or out by a stranger.
#[sqlx::test(migrations = "../../migrations")]
async fn another_tenants_folder_is_not_found(pool: PgPool) -> sqlx::Result<()> {
    let a = tenant(&pool, "folders-tenant-a").await;
    let b = tenant(&pool, "folders-tenant-b").await;
    folder(&pool, a, "uf-a", None, "Mine").await;
    add(&pool, "up-a", a, "a.csv").await;
    add(&pool, "up-b", b, "b.csv").await;

    assert!(list(&pool, b).await.unwrap().is_empty());
    assert!(!folder_in_tenant(&pool, b, "uf-a").await.unwrap());
    assert!(folder_in_tenant(&pool, a, "uf-a").await.unwrap());
    assert!(matches!(
        create(&pool, b, "uf-x", Some("uf-a"), "Child", "T").await,
        Err(FolderError::NotFound)
    ));
    assert!(matches!(
        update(
            &pool,
            b,
            "uf-a",
            FolderPatch {
                name: Some("Stolen"),
                ..FolderPatch::default()
            }
        )
        .await,
        Err(FolderError::NotFound)
    ));
    assert!(matches!(
        delete(&pool, b, "uf-a").await,
        Err(FolderError::NotFound)
    ));
    assert!(matches!(
        move_uploads(&pool, b, &["up-b".to_owned()], Some("uf-a")).await,
        Err(FolderError::NotFound)
    ));
    // Tenant A's upload is not B's to move either; nothing moved.
    assert!(matches!(
        move_uploads(&pool, b, &["up-a".to_owned()], None).await,
        Err(FolderError::NotFound)
    ));
    assert_eq!(list(&pool, a).await.unwrap()[0].name, "Mine");
    Ok(())
}

/// A move of several files is all or nothing: one id of another tenant (or none)
/// and no file moves.
#[sqlx::test(migrations = "../../migrations")]
async fn a_move_of_several_files_is_all_or_nothing(pool: PgPool) -> sqlx::Result<()> {
    let a = tenant(&pool, "folders-bulk-a").await;
    let b = tenant(&pool, "folders-bulk-b").await;
    folder(&pool, a, "uf-a", None, "A").await;
    add(&pool, "up-1", a, "1.csv").await;
    add(&pool, "up-2", a, "2.csv").await;
    add(&pool, "up-other", b, "x.csv").await;

    let all = ["up-1".to_owned(), "up-2".to_owned(), "up-other".to_owned()];
    assert!(matches!(
        move_uploads(&pool, a, &all, Some("uf-a")).await,
        Err(FolderError::NotFound)
    ));
    assert!(
        get(&pool, "up-1")
            .await
            .unwrap()
            .unwrap()
            .folder_id
            .is_none()
    );

    let moved = move_uploads(&pool, a, &all[..2], Some("uf-a"))
        .await
        .unwrap();
    assert_eq!(moved, 2);
    // The same id twice counts once.
    let twice = ["up-1".to_owned(), "up-1".to_owned()];
    assert_eq!(move_uploads(&pool, a, &twice, None).await.unwrap(), 1);
    assert_eq!(
        get(&pool, "up-2")
            .await
            .unwrap()
            .unwrap()
            .folder_id
            .as_deref(),
        Some("uf-a")
    );
    Ok(())
}

/// Moving a file does not touch `updated_at`, which dates a load's claim.
#[sqlx::test(migrations = "../../migrations")]
async fn moving_a_file_leaves_updated_at_alone(pool: PgPool) -> sqlx::Result<()> {
    let t = tenant(&pool, "folders-updated-at").await;
    folder(&pool, t, "uf-a", None, "A").await;
    let before = add(&pool, "up-1", t, "1.csv").await;
    move_uploads(&pool, t, &["up-1".to_owned()], Some("uf-a"))
        .await
        .unwrap();
    let after = get(&pool, "up-1").await.unwrap().unwrap();
    assert_eq!(after.updated_at, before.updated_at);
    assert_eq!(after.folder_id.as_deref(), Some("uf-a"));
    Ok(())
}

/// The listing filters narrow before the window is cut: a folder's files, the
/// files in no folder, and a search over file and table names with the user's
/// `%` and `_` matching themselves.
#[sqlx::test(migrations = "../../migrations")]
async fn the_listing_can_be_narrowed_by_folder_and_by_text(pool: PgPool) -> sqlx::Result<()> {
    let t = tenant(&pool, "folders-filter").await;
    let other = tenant(&pool, "folders-filter-other").await;
    folder(&pool, t, "uf-a", None, "A").await;
    add(&pool, "up-1", t, "Stock_2026.csv").await;
    add(&pool, "up-2", t, "stockX2026.csv").await;
    add(&pool, "up-3", t, "orders.csv").await;
    add(&pool, "up-4", other, "Stock_2026.csv").await;
    move_uploads(&pool, t, &["up-1".to_owned()], Some("uf-a"))
        .await
        .unwrap();

    assert_eq!(
        show(&pool, t, FolderFilter::Any, None).await,
        ["up-3", "up-2", "up-1"]
    );
    assert_eq!(
        show(&pool, t, FolderFilter::In("uf-a"), None).await,
        ["up-1"]
    );
    assert_eq!(
        show(&pool, t, FolderFilter::NoFolder, None).await,
        ["up-3", "up-2"]
    );
    assert_eq!(
        show(&pool, t, FolderFilter::Any, Some("STOCK_")).await,
        ["up-1"]
    );
    assert_eq!(
        show(&pool, t, FolderFilter::Any, Some("%")).await,
        Vec::<String>::new()
    );
    assert_eq!(
        show(&pool, t, FolderFilter::NoFolder, Some("stock")).await,
        ["up-2"],
        "folder and text together"
    );
    // Another tenant's folder id shows nothing, and names no oracle.
    let foreign = list_filtered(
        &pool,
        other,
        100,
        &ListFilter {
            folder: FolderFilter::In("uf-a"),
            text: None,
        },
    )
    .await
    .unwrap();
    assert!(foreign.is_empty());
    Ok(())
}

/// A grouped load puts every file in the folder and the table's claim remembers
/// it; a load with no folder leaves both as they were.
#[sqlx::test(migrations = "../../migrations")]
async fn a_load_puts_every_file_in_the_folder_and_the_claim_remembers_it(
    pool: PgPool,
) -> sqlx::Result<()> {
    let t = tenant(&pool, "folders-load").await;
    folder(&pool, t, "uf-a", None, "A").await;
    for id in ["up-1", "up-2", "up-3"] {
        add(&pool, id, t, "f.csv").await;
    }
    assert!(claim_table(&pool, t, "stock_raw", "up-1").await.unwrap());
    assert_eq!(claim_folder(&pool, t, "stock_raw").await.unwrap(), None);

    let options = json!({ "encoding": "utf-8", "delimiter": ",", "headerRow": 0 });
    let limits = LoadLimits {
        per_user: 10,
        total: 10,
    };
    let claim = |folder_id: Option<&'static str>| LoadClaim {
        parse_options: &options,
        bronze_table: "stock_raw",
        load_mode: LoadMode::Append,
        column_spec: None,
        on_bad_value: None,
        folder_id,
    };
    let outcome = mark_group_within_limits(
        &pool,
        "up-1",
        &["up-2".to_owned()],
        &claim(Some("uf-a")),
        limits,
    )
    .await
    .unwrap();
    assert!(matches!(outcome, MarkOutcome::Marked(_)));
    for id in ["up-1", "up-2"] {
        assert_eq!(
            get(&pool, id).await.unwrap().unwrap().folder_id.as_deref(),
            Some("uf-a"),
            "{id}"
        );
    }
    assert_eq!(get(&pool, "up-3").await.unwrap().unwrap().folder_id, None);
    assert_eq!(
        claim_folder(&pool, t, "stock_raw")
            .await
            .unwrap()
            .as_deref(),
        Some("uf-a")
    );
    // Another tenant reads no folder from this claim.
    let other = tenant(&pool, "folders-load-other").await;
    assert_eq!(claim_folder(&pool, other, "stock_raw").await.unwrap(), None);

    // A later load that names no folder changes neither the files nor the claim.
    let again = mark_group_within_limits(&pool, "up-3", &[], &claim(None), limits)
        .await
        .unwrap();
    assert!(matches!(again, MarkOutcome::Marked(_)));
    assert_eq!(get(&pool, "up-3").await.unwrap().unwrap().folder_id, None);
    assert_eq!(
        claim_folder(&pool, t, "stock_raw")
            .await
            .unwrap()
            .as_deref(),
        Some("uf-a")
    );
    Ok(())
}

/// Deleting a folder forgets it on the claim and never blocks on it.
#[sqlx::test(migrations = "../../migrations")]
async fn deleting_a_folder_clears_it_from_the_claim(pool: PgPool) -> sqlx::Result<()> {
    let t = tenant(&pool, "folders-claim-clear").await;
    folder(&pool, t, "uf-a", None, "A").await;
    claim_table(&pool, t, "stock_raw", "up-1").await.unwrap();
    sqlx::query(
        "UPDATE upload_table_claim SET folder_id = 'uf-a' WHERE bronze_table = 'stock_raw'",
    )
    .execute(&pool)
    .await?;

    delete(&pool, t, "uf-a").await.unwrap();
    assert_eq!(claim_folder(&pool, t, "stock_raw").await.unwrap(), None);
    Ok(())
}

/// The migration applies on a database that already holds uploads, and an
/// existing upload keeps a null folder.
#[sqlx::test(migrations = false)]
async fn an_upload_that_existed_before_the_migration_has_no_folder(
    pool: PgPool,
) -> sqlx::Result<()> {
    let before: Vec<_> = MIGRATOR
        .iter()
        .filter(|m| m.version < 112)
        .cloned()
        .collect();
    let first = Migrator {
        migrations: Cow::Owned(before),
        ..Migrator::DEFAULT
    };
    first.run(&pool).await?;
    let t = tenant(&pool, "folders-migration").await;
    sqlx::query(
        "INSERT INTO file_upload (id, original_filename, storage_key, content_type, size_bytes, \
         sha256, uploaded_by, tenant_id) VALUES ('up-old', 'old.csv', 'k', 'text/csv', 1, 'h', 'T', $1)",
    )
    .bind(t)
    .execute(&pool)
    .await?;

    MIGRATOR.run(&pool).await?;

    let old = get(&pool, "up-old").await.unwrap().unwrap();
    assert_eq!(old.folder_id, None);
    assert!(list(&pool, t).await.unwrap().is_empty());
    Ok(())
}
