//! Integration tests for `lakehouse_store::dashboard_access` against a real
//! Postgres (`#[sqlx::test]`, see `tests/reporting_settings.rs` for how the
//! database is provided). `BI-19` part B1, T4.

#![allow(clippy::unwrap_used, clippy::expect_used)]

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap runs for this test binary.
use lakehouse_test_support as _;

use lakehouse_store::StoreError;
use lakehouse_store::dashboard_access::{
    NewRule, all_restrictions, delete_item, replace_item, restriction_for_item, rules_for_grantee,
    rules_for_item, rules_for_items,
};
use sqlx::PgPool;
use uuid::Uuid;

async fn user(pool: &PgPool, name: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO app_user (name, email) VALUES ($1, $2) RETURNING id")
        .bind(name)
        .bind(format!("{name}@example.test"))
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn role(pool: &PgPool, name: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO role (name) VALUES ($1) RETURNING id")
        .bind(name)
        .fetch_one(pool)
        .await
        .unwrap()
}

fn for_user(id: Uuid, level: &str) -> NewRule {
    NewRule {
        user_id: Some(id),
        role_id: None,
        level: level.to_owned(),
    }
}

fn for_role(id: Uuid, level: &str) -> NewRule {
    NewRule {
        user_id: None,
        role_id: Some(id),
        level: level.to_owned(),
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_rule_names_exactly_one_grantee_and_a_known_level(pool: PgPool) -> sqlx::Result<()> {
    let u = user(&pool, "ann").await;
    let r = role(&pool, "readers").await;
    let both = NewRule {
        user_id: Some(u),
        role_id: Some(r),
        level: "view".to_owned(),
    };
    let neither = NewRule {
        user_id: None,
        role_id: None,
        level: "view".to_owned(),
    };
    for bad in [both, neither, for_user(u, "owner"), for_user(u, "")] {
        assert!(
            replace_item(
                &pool,
                "board",
                "b1",
                std::slice::from_ref(&bad),
                false,
                None
            )
            .await
            .is_err(),
            "{bad:?} should be refused by the table's CHECKs"
        );
    }
    assert!(
        replace_item(
            &pool,
            "dashboard",
            "b1",
            &[for_user(u, "view")],
            false,
            None
        )
        .await
        .is_err(),
        "an item kind outside board and folder is refused"
    );
    assert_eq!(rules_for_item(&pool, "board", "b1").await.unwrap().len(), 0);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_same_grantee_twice_on_one_item_is_a_conflict(pool: PgPool) -> sqlx::Result<()> {
    let u = user(&pool, "ann").await;
    let r = role(&pool, "readers").await;
    let err = replace_item(
        &pool,
        "board",
        "b1",
        &[for_user(u, "view"), for_user(u, "edit")],
        false,
        None,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, StoreError::Conflict), "{err:?}");
    let err = replace_item(
        &pool,
        "board",
        "b1",
        &[for_role(r, "view"), for_role(r, "manage")],
        false,
        None,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, StoreError::Conflict), "{err:?}");
    // The failed replacement left nothing behind (one transaction).
    assert_eq!(rules_for_item(&pool, "board", "b1").await.unwrap().len(), 0);
    // The same person on a board and on a folder with the same id is fine:
    // the kind is part of the key.
    replace_item(&pool, "board", "x", &[for_user(u, "view")], false, None)
        .await
        .unwrap();
    replace_item(&pool, "folder", "x", &[for_user(u, "view")], false, None)
        .await
        .unwrap();
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_deleted_users_rules_go_with_them_and_a_role_blocks_nothing_else(
    pool: PgPool,
) -> sqlx::Result<()> {
    let ann = user(&pool, "ann").await;
    let bob = user(&pool, "bob").await;
    let r = role(&pool, "readers").await;
    replace_item(
        &pool,
        "board",
        "b1",
        &[
            for_user(ann, "view"),
            for_user(bob, "edit"),
            for_role(r, "view"),
        ],
        false,
        Some(bob),
    )
    .await
    .unwrap();
    sqlx::query("DELETE FROM app_user WHERE id = $1")
        .bind(ann)
        .execute(&pool)
        .await?;
    let left = rules_for_item(&pool, "board", "b1").await.unwrap();
    assert_eq!(left.len(), 2, "ann's rule went with her: {left:?}");
    assert!(left.iter().all(|rule| rule.user_id != Some(ann)));

    // The grantor leaving keeps the rule (granted_by follows to NULL).
    sqlx::query("DELETE FROM app_user WHERE id = $1")
        .bind(bob)
        .execute(&pool)
        .await?;
    let left = rules_for_item(&pool, "board", "b1").await.unwrap();
    assert_eq!(left.len(), 1, "only the role's rule is left: {left:?}");
    assert_eq!(left[0].role_id, Some(r));
    assert_eq!(left[0].granted_by, None);

    // A deleted role's rules go too.
    sqlx::query("DELETE FROM role WHERE id = $1")
        .bind(r)
        .execute(&pool)
        .await?;
    assert_eq!(rules_for_item(&pool, "board", "b1").await.unwrap().len(), 0);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn replacing_an_item_swaps_its_rules_and_its_mark_in_one_go(
    pool: PgPool,
) -> sqlx::Result<()> {
    let ann = user(&pool, "ann").await;
    let bob = user(&pool, "bob").await;
    replace_item(
        &pool,
        "folder",
        "f1",
        &[for_user(ann, "view")],
        true,
        Some(bob),
    )
    .await
    .unwrap();
    let mark = restriction_for_item(&pool, "folder", "f1").await.unwrap();
    assert_eq!(mark.expect("marked").restricted_by, Some(bob));

    replace_item(
        &pool,
        "folder",
        "f1",
        &[for_user(bob, "manage")],
        false,
        Some(ann),
    )
    .await
    .unwrap();
    let rules = rules_for_item(&pool, "folder", "f1").await.unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].user_id, Some(bob));
    assert_eq!(rules[0].level, "manage");
    assert_eq!(rules[0].granted_by, Some(ann));
    assert!(
        restriction_for_item(&pool, "folder", "f1")
            .await
            .unwrap()
            .is_none(),
        "the mark was cleared by the second replacement"
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_failed_replacement_keeps_what_was_there(pool: PgPool) -> sqlx::Result<()> {
    let ann = user(&pool, "ann").await;
    replace_item(&pool, "board", "b1", &[for_user(ann, "view")], true, None)
        .await
        .unwrap();
    // The second rule names a user that does not exist.
    let err = replace_item(
        &pool,
        "board",
        "b1",
        &[for_user(ann, "edit"), for_user(Uuid::new_v4(), "view")],
        false,
        None,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, StoreError::ForeignKeyViolation), "{err:?}");
    let rules = rules_for_item(&pool, "board", "b1").await.unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].level, "view", "the old rule is still there");
    assert!(
        restriction_for_item(&pool, "board", "b1")
            .await
            .unwrap()
            .is_some(),
        "and so is the old mark"
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_person_is_found_by_their_own_rules_and_those_of_their_roles(
    pool: PgPool,
) -> sqlx::Result<()> {
    let ann = user(&pool, "ann").await;
    let bob = user(&pool, "bob").await;
    let readers = role(&pool, "readers").await;
    let others = role(&pool, "others").await;
    replace_item(&pool, "board", "b1", &[for_user(ann, "view")], false, None)
        .await
        .unwrap();
    replace_item(
        &pool,
        "board",
        "b2",
        &[for_role(readers, "edit")],
        false,
        None,
    )
    .await
    .unwrap();
    replace_item(
        &pool,
        "folder",
        "f1",
        &[for_user(bob, "manage"), for_role(others, "view")],
        false,
        None,
    )
    .await
    .unwrap();
    let mut mine: Vec<_> = rules_for_grantee(&pool, ann, &[readers])
        .await
        .unwrap()
        .into_iter()
        .map(|r| (r.item_kind, r.item_id, r.level))
        .collect();
    mine.sort();
    assert_eq!(
        mine,
        vec![
            ("board".to_owned(), "b1".to_owned(), "view".to_owned()),
            ("board".to_owned(), "b2".to_owned(), "edit".to_owned()),
        ]
    );
    assert!(
        rules_for_grantee(&pool, ann, &[]).await.unwrap().len() == 1,
        "without the role only her own rule"
    );
    let on_folders = rules_for_items(&pool, "folder", &["f1".to_owned(), "f9".to_owned()])
        .await
        .unwrap();
    assert_eq!(on_folders.len(), 2);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn purging_an_item_drops_its_rules_and_its_mark_and_no_one_elses(
    pool: PgPool,
) -> sqlx::Result<()> {
    let ann = user(&pool, "ann").await;
    replace_item(&pool, "board", "b1", &[for_user(ann, "view")], true, None)
        .await
        .unwrap();
    replace_item(&pool, "board", "b2", &[for_user(ann, "view")], true, None)
        .await
        .unwrap();
    delete_item(&pool, "board", "b1").await.unwrap();
    assert_eq!(rules_for_item(&pool, "board", "b1").await.unwrap().len(), 0);
    assert_eq!(rules_for_item(&pool, "board", "b2").await.unwrap().len(), 1);
    let marks = all_restrictions(&pool).await.unwrap();
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0].item_id, "b2");
    Ok(())
}
