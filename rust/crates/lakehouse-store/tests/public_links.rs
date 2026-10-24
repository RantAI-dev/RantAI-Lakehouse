//! Integration tests for `lakehouse_store::public_links` against a real
//! Postgres (`#[sqlx::test]`, see `tests/reporting_settings.rs` for how the
//! database is provided). `BI-19` part A, T1.

#![allow(clippy::unwrap_used, clippy::expect_used)]

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap runs for this test binary.
use lakehouse_test_support as _;

use lakehouse_store::public_links::{
    PasswordChange, copy_legacy, delete, get_for_board, get_live_by_token, issue_ticket,
    live_tokens, replace, ticket_is_live, update,
};
use sqlx::PgPool;
use time::{Duration, OffsetDateTime};

fn in_days(days: i64) -> OffsetDateTime {
    OffsetDateTime::now_utc() + Duration::days(days)
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_dashboard_has_one_link_and_a_new_one_ends_the_old(pool: PgPool) -> sqlx::Result<()> {
    let first = replace(&pool, "b_1", "p_first_token", in_days(30), None, None)
        .await
        .unwrap();
    assert_eq!(first.token, "p_first_token");
    assert!(!first.has_password());

    let second = replace(
        &pool,
        "b_1",
        "p_second_token",
        in_days(7),
        Some("hash"),
        None,
    )
    .await
    .unwrap();
    assert_ne!(first.token, second.token);
    assert!(second.has_password());
    assert!(
        get_live_by_token(&pool, "p_first_token")
            .await
            .unwrap()
            .is_none(),
        "the old token stops at once"
    );
    assert!(
        get_live_by_token(&pool, "p_second_token")
            .await
            .unwrap()
            .is_some()
    );
    let (rows,): (i64,) = sqlx::query_as("SELECT count(*) FROM dashboard_public_link")
        .fetch_one(&pool)
        .await?;
    assert_eq!(rows, 1);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_expired_link_is_not_returned_by_the_live_lookup_but_is_still_listed_for_its_board(
    pool: PgPool,
) -> sqlx::Result<()> {
    replace(&pool, "b_1", "p_old_token_x", in_days(30), None, None)
        .await
        .unwrap();
    sqlx::query("UPDATE dashboard_public_link SET expires_at = now() - interval '1 second'")
        .execute(&pool)
        .await?;
    assert!(
        get_live_by_token(&pool, "p_old_token_x")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        get_live_by_token(&pool, "p_unknown_tk")
            .await
            .unwrap()
            .is_none()
    );
    assert!(live_tokens(&pool).await.unwrap().is_empty());
    assert!(
        get_for_board(&pool, "b_1").await.unwrap().is_some(),
        "the Share dialog can still say it has expired"
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn updating_keeps_the_token_and_refuses_an_expired_link(pool: PgPool) -> sqlx::Result<()> {
    replace(&pool, "b_1", "p_keep_token", in_days(1), None, None)
        .await
        .unwrap();
    let later = in_days(90);
    let changed = update(&pool, "b_1", later, &PasswordChange::Keep)
        .await
        .unwrap()
        .expect("a live link is updated");
    assert_eq!(changed.token, "p_keep_token");
    assert_eq!(changed.password_version, 0, "keeping does not bump");
    assert!(changed.expires_at > in_days(80));

    sqlx::query("UPDATE dashboard_public_link SET expires_at = now() - interval '1 second'")
        .execute(&pool)
        .await?;
    assert!(
        update(&pool, "b_1", later, &PasswordChange::Keep)
            .await
            .unwrap()
            .is_none(),
        "an expired link is not revived"
    );
    assert!(
        update(&pool, "b_none", later, &PasswordChange::Remove)
            .await
            .unwrap()
            .is_none()
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_password_change_bumps_the_version_and_kills_the_tickets(
    pool: PgPool,
) -> sqlx::Result<()> {
    let link = replace(&pool, "b_1", "p_pw_token_1", in_days(30), Some("h1"), None)
        .await
        .unwrap();
    issue_ticket(&pool, "b_1", link.password_version, "t_hash_a")
        .await
        .unwrap()
        .expect("a ticket for the current password");
    assert!(ticket_is_live(&pool, "b_1", "t_hash_a").await.unwrap());
    assert!(!ticket_is_live(&pool, "b_other", "t_hash_a").await.unwrap());
    assert!(
        !ticket_is_live(&pool, "b_1", "t_hash_unknown")
            .await
            .unwrap()
    );

    let changed = update(
        &pool,
        "b_1",
        in_days(30),
        &PasswordChange::Set("h2".to_owned()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(changed.password_version, 1);
    assert_eq!(changed.password_hash.as_deref(), Some("h2"));
    assert!(
        !ticket_is_live(&pool, "b_1", "t_hash_a").await.unwrap(),
        "the old ticket names the old password"
    );
    assert!(
        issue_ticket(&pool, "b_1", 0, "t_hash_b")
            .await
            .unwrap()
            .is_none(),
        "no ticket is issued for a password that has been replaced"
    );

    let removed = update(&pool, "b_1", in_days(30), &PasswordChange::Remove)
        .await
        .unwrap()
        .unwrap();
    assert!(!removed.has_password());
    assert_eq!(removed.password_version, 2);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_ticket_never_outlives_its_link_and_ending_the_link_deletes_the_tickets(
    pool: PgPool,
) -> sqlx::Result<()> {
    let soon = OffsetDateTime::now_utc() + Duration::hours(2);
    replace(&pool, "b_1", "p_short_link", soon, Some("h"), None)
        .await
        .unwrap();
    let ends = issue_ticket(&pool, "b_1", 0, "t_hash_c")
        .await
        .unwrap()
        .unwrap();
    assert!(
        ends <= soon + Duration::seconds(1),
        "capped at the link's expiry"
    );
    assert!(ends > OffsetDateTime::now_utc() + Duration::hours(1));

    sqlx::query("UPDATE dashboard_public_link_ticket SET expires_at = now() - interval '1 second'")
        .execute(&pool)
        .await?;
    assert!(!ticket_is_live(&pool, "b_1", "t_hash_c").await.unwrap());

    replace(&pool, "b_2", "p_other_link", in_days(30), Some("h"), None)
        .await
        .unwrap();
    issue_ticket(&pool, "b_2", 0, "t_hash_d")
        .await
        .unwrap()
        .unwrap();
    assert!(delete(&pool, "b_2").await.unwrap());
    assert!(!delete(&pool, "b_2").await.unwrap(), "nothing left to end");
    let (left,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM dashboard_public_link_ticket WHERE board_id = 'b_2'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(left, 0, "the tickets went with the link");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_replaced_link_takes_its_tickets_with_it(pool: PgPool) -> sqlx::Result<()> {
    replace(&pool, "b_1", "p_first_aaaa", in_days(30), Some("h"), None)
        .await
        .unwrap();
    issue_ticket(&pool, "b_1", 0, "t_hash_e")
        .await
        .unwrap()
        .unwrap();
    replace(&pool, "b_1", "p_second_aaa", in_days(30), Some("h"), None)
        .await
        .unwrap();
    assert!(!ticket_is_live(&pool, "b_1", "t_hash_e").await.unwrap());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_copy_of_old_links_is_idempotent_and_gives_thirty_days(
    pool: PgPool,
) -> sqlx::Result<()> {
    let old = vec![
        ("b_1".to_owned(), "p_legacy_one".to_owned()),
        ("b_2".to_owned(), "p_legacy_two".to_owned()),
    ];
    assert_eq!(copy_legacy(&pool, &old).await.unwrap(), 2);
    assert_eq!(
        copy_legacy(&pool, &old).await.unwrap(),
        0,
        "a second run copies nothing"
    );
    let (rows,): (i64,) = sqlx::query_as("SELECT count(*) FROM dashboard_public_link")
        .fetch_one(&pool)
        .await?;
    assert_eq!(rows, 2);
    let link = get_live_by_token(&pool, "p_legacy_one")
        .await
        .unwrap()
        .unwrap();
    assert!(link.expires_at > in_days(29) && link.expires_at < in_days(31));
    assert!(!link.has_password());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_ended_link_does_not_come_back_when_the_copy_is_asked_again(
    pool: PgPool,
) -> sqlx::Result<()> {
    // The old column is never cleared, so a second start reads the same
    // tokens again. The marker is what keeps an ended link ended.
    let old = vec![("b_1".to_owned(), "p_legacy_one".to_owned())];
    copy_legacy(&pool, &old).await.unwrap();
    delete(&pool, "b_1").await.unwrap();
    assert_eq!(copy_legacy(&pool, &old).await.unwrap(), 0);
    assert!(get_for_board(&pool, "b_1").await.unwrap().is_none());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_copy_leaves_a_link_a_board_already_has(pool: PgPool) -> sqlx::Result<()> {
    replace(&pool, "b_1", "p_made_after", in_days(7), None, None)
        .await
        .unwrap();
    let old = vec![("b_1".to_owned(), "p_legacy_one".to_owned())];
    assert_eq!(copy_legacy(&pool, &old).await.unwrap(), 0);
    assert_eq!(
        get_for_board(&pool, "b_1").await.unwrap().unwrap().token,
        "p_made_after"
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_deleted_user_leaves_the_link_in_place(pool: PgPool) -> sqlx::Result<()> {
    let user: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO app_user (name, email) VALUES ('a', 'a@example.com') RETURNING id",
    )
    .fetch_one(&pool)
    .await?;
    replace(&pool, "b_1", "p_owned_link", in_days(7), None, Some(user))
        .await
        .unwrap();
    sqlx::query("DELETE FROM app_user WHERE id = $1")
        .bind(user)
        .execute(&pool)
        .await?;
    let link = get_for_board(&pool, "b_1").await.unwrap().unwrap();
    assert_eq!(link.created_by, None);
    Ok(())
}

#[test]
fn a_debug_render_shows_neither_the_token_nor_the_hash() {
    let link = lakehouse_store::public_links::PublicLink {
        board_id: "b_1".to_owned(),
        token: "p_secret_token".to_owned(),
        expires_at: OffsetDateTime::now_utc(),
        password_hash: Some("$argon2id$secret".to_owned()),
        password_version: 0,
        created_by: None,
        created_at: OffsetDateTime::now_utc(),
        updated_at: OffsetDateTime::now_utc(),
    };
    let text = format!(
        "{link:?} {:?}",
        PasswordChange::Set("$argon2id$secret".to_owned())
    );
    assert!(!text.contains("p_secret_token"));
    assert!(!text.contains("argon2id"));
}

/// `BI-19` part A review (NOTE): simultaneous "new link" calls for one
/// dashboard all succeed and leave one link.
#[sqlx::test(migrations = "../../migrations")]
async fn simultaneous_new_links_for_one_dashboard_all_succeed_and_leave_one(
    pool: PgPool,
) -> sqlx::Result<()> {
    let pool = std::sync::Arc::new(pool);
    let mut tasks = Vec::new();
    for i in 0..10 {
        let pool = std::sync::Arc::clone(&pool);
        tasks.push(tokio::spawn(async move {
            replace(
                &pool,
                "b_1",
                &format!("p_concurrent_{i}"),
                in_days(7),
                None,
                None,
            )
            .await
            .unwrap()
        }));
    }
    for t in tasks {
        t.await.unwrap();
    }
    let (rows,): (i64,) = sqlx::query_as("SELECT count(*) FROM dashboard_public_link")
        .fetch_one(&*pool)
        .await?;
    assert_eq!(rows, 1);
    Ok(())
}
