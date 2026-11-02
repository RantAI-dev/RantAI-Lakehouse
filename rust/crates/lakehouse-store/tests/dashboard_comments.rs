//! Integration tests for `lakehouse_store::dashboard_comments` against a real
//! Postgres (`#[sqlx::test]`). `BI-19` part E, T17.

#![allow(clippy::unwrap_used, clippy::expect_used)]

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap runs for this test binary.
use lakehouse_test_support as _;

use lakehouse_store::dashboard_comments::{
    delete_for_board, get, insert, list, mark_mentions_read, mentions_of, soft_delete,
    unread_mentions, update,
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

#[sqlx::test(migrations = "../../migrations")]
async fn a_comment_is_listed_oldest_first_with_its_author_and_mentions(
    pool: PgPool,
) -> sqlx::Result<()> {
    let ann = user(&pool, "ann").await;
    let bob = user(&pool, "bob").await;
    let first = insert(&pool, "b1", ann, "first", &[bob]).await.unwrap().0;
    let second = insert(&pool, "b1", bob, "second", &[]).await.unwrap().0;
    insert(&pool, "b2", bob, "elsewhere", &[]).await.unwrap();

    let (rows, truncated) = list(&pool, "b1").await.unwrap();
    assert!(!truncated);
    assert_eq!(
        rows.iter().map(|c| c.id).collect::<Vec<_>>(),
        [first, second]
    );
    assert_eq!(rows[0].author_name.as_deref(), Some("ann"));
    let mentioned = mentions_of(&pool, &[first, second]).await.unwrap();
    assert_eq!(mentioned, [(first, bob, "bob".to_owned())]);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_database_refuses_an_empty_or_oversized_text(pool: PgPool) -> sqlx::Result<()> {
    let ann = user(&pool, "ann").await;
    assert!(insert(&pool, "b1", ann, "", &[]).await.is_err());
    assert!(
        insert(&pool, "b1", ann, &"x".repeat(2001), &[])
            .await
            .is_err()
    );
    assert!(
        insert(&pool, "b1", ann, &"x".repeat(2000), &[])
            .await
            .is_ok()
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_edit_notifies_only_the_people_newly_mentioned(pool: PgPool) -> sqlx::Result<()> {
    let ann = user(&pool, "ann").await;
    let bob = user(&pool, "bob").await;
    let cy = user(&pool, "cy").await;
    let id = insert(&pool, "b1", ann, "hi", &[bob]).await.unwrap().0;
    mark_mentions_read(&pool, bob).await.unwrap();

    let added = update(&pool, id, "hi again", &[bob, cy]).await.unwrap();
    assert_eq!(added, Some(vec![cy]));
    assert_eq!(
        unread_mentions(&pool, bob).await.unwrap().len(),
        0,
        "bob stays read"
    );
    assert_eq!(unread_mentions(&pool, cy).await.unwrap().len(), 1);
    let comment = get(&pool, id).await.unwrap().unwrap();
    assert_eq!(comment.body, "hi again");
    assert!(comment.edited_at.is_some());

    update(&pool, id, "no one", &[]).await.unwrap();
    assert_eq!(mentions_of(&pool, &[id]).await.unwrap().len(), 0);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_deleted_comment_cannot_be_read_edited_or_deleted_again(
    pool: PgPool,
) -> sqlx::Result<()> {
    let ann = user(&pool, "ann").await;
    let bob = user(&pool, "bob").await;
    let id = insert(&pool, "b1", ann, "gone", &[bob]).await.unwrap().0;

    assert!(soft_delete(&pool, id, ann).await.unwrap());
    assert!(!soft_delete(&pool, id, ann).await.unwrap());
    assert!(get(&pool, id).await.unwrap().is_none());
    assert_eq!(list(&pool, "b1").await.unwrap().0.len(), 0);
    assert_eq!(update(&pool, id, "back", &[]).await.unwrap(), None);
    assert_eq!(
        unread_mentions(&pool, bob).await.unwrap().len(),
        0,
        "a deleted comment does not stay in the bell"
    );
    let (by,): (Option<Uuid>,) =
        sqlx::query_as("SELECT deleted_by FROM dashboard_comment WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(by, Some(ann));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn mentions_are_read_once_and_a_purged_board_loses_its_comments(
    pool: PgPool,
) -> sqlx::Result<()> {
    let ann = user(&pool, "ann").await;
    let bob = user(&pool, "bob").await;
    insert(&pool, "b1", ann, "one", &[bob]).await.unwrap();
    insert(&pool, "b2", ann, "two", &[bob]).await.unwrap();
    assert_eq!(unread_mentions(&pool, bob).await.unwrap().len(), 2);
    assert_eq!(mark_mentions_read(&pool, bob).await.unwrap(), 2);
    assert_eq!(unread_mentions(&pool, bob).await.unwrap().len(), 0);

    assert_eq!(delete_for_board(&pool, "b1").await.unwrap(), 1);
    assert_eq!(list(&pool, "b1").await.unwrap().0.len(), 0);
    assert_eq!(list(&pool, "b2").await.unwrap().0.len(), 1);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_deleted_account_leaves_its_comments_without_an_author(pool: PgPool) -> sqlx::Result<()> {
    let ann = user(&pool, "ann").await;
    let bob = user(&pool, "bob").await;
    let id = insert(&pool, "b1", ann, "hello", &[bob]).await.unwrap().0;
    sqlx::query("DELETE FROM app_user WHERE id = $1")
        .bind(ann)
        .execute(&pool)
        .await?;
    let comment = get(&pool, id).await.unwrap().unwrap();
    assert_eq!((comment.author_id, comment.author_name), (None, None));
    sqlx::query("DELETE FROM app_user WHERE id = $1")
        .bind(bob)
        .execute(&pool)
        .await?;
    assert_eq!(mentions_of(&pool, &[id]).await.unwrap().len(), 0);
    Ok(())
}

/// `BI-19` E review E-1: a mention emails a person only when they have no other
/// unread mention on that dashboard; reading them (the bell) starts it again.
#[sqlx::test(migrations = "../../migrations")]
async fn one_email_per_person_per_dashboard_until_it_is_read(pool: PgPool) -> sqlx::Result<()> {
    let ann = user(&pool, "ann").await;
    let bob = user(&pool, "bob").await;
    let cy = user(&pool, "cy").await;

    let (first, email) = insert(&pool, "b1", ann, "one", &[bob]).await.unwrap();
    assert_eq!(email, [bob], "the first mention emails");
    let (_, email) = insert(&pool, "b1", ann, "two", &[bob, cy]).await.unwrap();
    assert_eq!(email, [cy], "bob has an unread mention here already");
    let (_, email) = insert(&pool, "b2", ann, "other board", &[bob])
        .await
        .unwrap();
    assert_eq!(email, [bob], "another dashboard counts on its own");
    assert_eq!(
        unread_mentions(&pool, bob).await.unwrap().len(),
        3,
        "the bell still lists every mention"
    );

    // An edit follows the same rule.
    let third = insert(&pool, "b3", ann, "three", &[]).await.unwrap().0;
    assert_eq!(
        update(&pool, third, "3", &[bob]).await.unwrap(),
        Some(vec![bob])
    );
    assert_eq!(
        update(&pool, first, "1", &[bob, cy]).await.unwrap(),
        Some(vec![])
    );

    mark_mentions_read(&pool, bob).await.unwrap();
    let (_, email) = insert(&pool, "b1", ann, "again", &[bob]).await.unwrap();
    assert_eq!(email, [bob], "read, so the next mention emails again");
    Ok(())
}

/// `BI-19` E review E-6c: the list says when older comments are left out.
#[sqlx::test(migrations = "../../migrations")]
async fn a_long_thread_is_marked_truncated(pool: PgPool) -> sqlx::Result<()> {
    let ann = user(&pool, "ann").await;
    sqlx::query(
        "INSERT INTO dashboard_comment (board_id, author_id, body) \
         SELECT 'b1', $1, 'c' || n FROM generate_series(1, 201) AS n",
    )
    .bind(ann)
    .execute(&pool)
    .await?;
    let (rows, truncated) = list(&pool, "b1").await.unwrap();
    assert_eq!(rows.len(), 200);
    assert!(truncated);
    Ok(())
}
