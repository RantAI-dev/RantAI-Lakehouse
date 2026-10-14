//! Integration tests for `lakehouse_store::chat_term` against a real
//! Postgres (`#[sqlx::test]`, see `tests/maintenance_policy.rs` for how the
//! database is provided).

#![allow(clippy::unwrap_used, clippy::expect_used)]

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap runs for this test binary.
use lakehouse_test_support as _;

use lakehouse_store::StoreError;
use lakehouse_store::chat_term::{MAX_TERMS_PER_OWNER, delete, list_for_owner, upsert};
use sqlx::PgPool;

fn terms_of(list: &[lakehouse_store::chat_term::ChatTerm]) -> Vec<&str> {
    list.iter().map(|t| t.term.as_str()).collect()
}

fn validation_message(err: StoreError) -> String {
    match err {
        StoreError::Validation(message) => message,
        other => panic!("expected a validation error, got {other:?}"),
    }
}

async fn fill(pool: &PgPool, owner: &str, count: usize) {
    for i in 0..count {
        upsert(pool, owner, &format!("term-{i}"), "a meaning", "")
            .await
            .expect("fill");
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_owner_with_no_terms_lists_nothing(pool: PgPool) -> sqlx::Result<()> {
    assert!(
        list_for_owner(&pool, "owner-a")
            .await
            .expect("list")
            .is_empty()
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn upsert_trims_and_lower_cases_the_term_and_a_second_upsert_replaces(
    pool: PgPool,
) -> sqlx::Result<()> {
    let stored = upsert(
        &pool,
        "owner-a",
        "  Net Revenue \t",
        "revenue after refunds",
        "show net revenue",
    )
    .await
    .expect("upsert");
    assert_eq!(stored.term, "net revenue");
    assert_eq!(stored.meaning, "revenue after refunds");
    assert_eq!(stored.question, "show net revenue");

    let again = upsert(&pool, "owner-a", "NET REVENUE", "revenue before tax", "")
        .await
        .expect("upsert again");
    assert_eq!(again.term, "net revenue");
    assert_eq!(again.meaning, "revenue before tax");
    assert_eq!(again.question, "");
    assert!(again.updated_at >= stored.updated_at);

    let listed = list_for_owner(&pool, "owner-a").await.expect("list");
    assert_eq!(listed, vec![again], "the second save replaces the row");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn terms_list_newest_first_and_an_updated_term_moves_to_the_top(
    pool: PgPool,
) -> sqlx::Result<()> {
    for term in ["alpha", "bravo", "charlie"] {
        upsert(&pool, "owner-a", term, "m", "")
            .await
            .expect("upsert");
    }
    let listed = list_for_owner(&pool, "owner-a").await.expect("list");
    assert_eq!(terms_of(&listed), ["charlie", "bravo", "alpha"]);

    upsert(&pool, "owner-a", "alpha", "m2", "")
        .await
        .expect("update");
    let listed = list_for_owner(&pool, "owner-a").await.expect("list");
    assert_eq!(terms_of(&listed), ["alpha", "charlie", "bravo"]);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn one_owner_never_reads_overwrites_or_deletes_another_owners_term(
    pool: PgPool,
) -> sqlx::Result<()> {
    let mine = upsert(&pool, "owner-a", "revenue", "net of refunds", "q")
        .await
        .expect("upsert a");

    assert!(
        list_for_owner(&pool, "owner-b")
            .await
            .expect("list b")
            .is_empty()
    );

    let theirs = upsert(&pool, "owner-b", "revenue", "gross sales", "")
        .await
        .expect("upsert b");
    assert_eq!(theirs.meaning, "gross sales");
    assert_eq!(
        list_for_owner(&pool, "owner-a").await.expect("list a"),
        vec![mine.clone()],
        "owner-b saving the same word must not overwrite owner-a's"
    );

    assert!(delete(&pool, "owner-b", "revenue").await.expect("delete b"));
    assert!(
        !delete(&pool, "owner-b", "revenue")
            .await
            .expect("delete b again"),
        "owner-b has nothing left to delete"
    );
    assert_eq!(
        list_for_owner(&pool, "owner-a").await.expect("list a"),
        vec![mine],
        "owner-b's delete must not touch owner-a's row"
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn delete_normalises_the_term_and_reports_whether_a_row_existed(
    pool: PgPool,
) -> sqlx::Result<()> {
    upsert(&pool, "owner-a", "Revenue", "m", "")
        .await
        .expect("upsert");
    assert!(
        delete(&pool, "owner-a", "  REVENUE ")
            .await
            .expect("delete")
    );
    assert!(
        list_for_owner(&pool, "owner-a")
            .await
            .expect("list")
            .is_empty()
    );
    assert!(
        !delete(&pool, "owner-a", "revenue")
            .await
            .expect("delete again")
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn each_bound_is_refused_naming_its_field_and_stores_nothing(
    pool: PgPool,
) -> sqlx::Result<()> {
    let sixty = "t".repeat(60);
    let two_hundred = "m".repeat(200);
    let five_hundred = "q".repeat(500);
    upsert(&pool, "owner-a", &sixty, &two_hundred, &five_hundred)
        .await
        .expect("the limits themselves are allowed");
    delete(&pool, "owner-a", &sixty).await.expect("clean up");

    let too_long_term = "t".repeat(61);
    let too_long_meaning = "m".repeat(201);
    let too_long_question = "q".repeat(501);
    let cases: [(&str, &str, &str, &str); 6] = [
        ("term", "", "m", ""),
        ("term", "   ", "m", ""),
        ("term", too_long_term.as_str(), "m", ""),
        ("meaning", "t", "", ""),
        ("meaning", "t", "   ", ""),
        ("meaning", "t", too_long_meaning.as_str(), ""),
    ];
    for (field, term, meaning, question) in cases {
        let message = validation_message(
            upsert(&pool, "owner-a", term, meaning, question)
                .await
                .expect_err("should be refused"),
        );
        assert!(
            message.starts_with(field),
            "{message:?} should name {field}"
        );
    }
    let message = validation_message(
        upsert(&pool, "owner-a", "t", "m", &too_long_question)
            .await
            .expect_err("question too long"),
    );
    assert!(message.starts_with("question"), "{message:?}");

    assert!(
        list_for_owner(&pool, "owner-a")
            .await
            .expect("list")
            .is_empty()
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_bounds_count_characters_not_bytes(pool: PgPool) -> sqlx::Result<()> {
    let sixty = "é".repeat(60);
    let stored = upsert(&pool, "owner-a", &sixty, &"é".repeat(200), "")
        .await
        .expect("60 two-byte characters fit");
    assert_eq!(stored.term.chars().count(), 60);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_101st_new_term_is_refused_but_an_existing_term_still_updates(
    pool: PgPool,
) -> sqlx::Result<()> {
    assert_eq!(MAX_TERMS_PER_OWNER, 100);
    fill(&pool, "owner-a", MAX_TERMS_PER_OWNER).await;

    let message = validation_message(
        upsert(&pool, "owner-a", "one too many", "m", "")
            .await
            .expect_err("the 101st term"),
    );
    assert!(message.contains("100"), "{message:?} should name the limit");
    assert_eq!(
        list_for_owner(&pool, "owner-a").await.expect("list").len(),
        MAX_TERMS_PER_OWNER
    );

    let updated = upsert(&pool, "owner-a", "Term-7", "a new meaning", "")
        .await
        .expect("an existing term still updates at the cap");
    assert_eq!(updated.meaning, "a new meaning");

    upsert(&pool, "owner-b", "one too many", "m", "")
        .await
        .expect("another owner's list is separate");

    assert!(delete(&pool, "owner-a", "term-0").await.expect("delete"));
    upsert(&pool, "owner-a", "one too many", "m", "")
        .await
        .expect("room again after a delete");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_new_terms_cannot_pass_the_cap_together(pool: PgPool) -> sqlx::Result<()> {
    fill(&pool, "owner-a", MAX_TERMS_PER_OWNER - 1).await;

    let handles: Vec<_> = (0..8)
        .map(|i| {
            let pool = pool.clone();
            tokio::spawn(
                async move { upsert(&pool, "owner-a", &format!("racer-{i}"), "m", "").await },
            )
        })
        .collect();
    let mut accepted = 0;
    for handle in handles {
        if handle.await.expect("task").is_ok() {
            accepted += 1;
        }
    }
    assert_eq!(accepted, 1, "exactly one racer fits under the cap");
    assert_eq!(
        list_for_owner(&pool, "owner-a").await.expect("list").len(),
        MAX_TERMS_PER_OWNER
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_table_itself_refuses_rows_written_around_the_store(pool: PgPool) -> sqlx::Result<()> {
    let long_term = "t".repeat(61);
    let long_meaning = "m".repeat(201);
    let long_question = "q".repeat(501);
    let cases: [(&str, &str, &str, &str, &str); 8] = [
        ("an upper-case term", "owner-a", "Revenue", "m", ""),
        ("an untrimmed term", "owner-a", " revenue", "m", ""),
        ("an empty term", "owner-a", "", "m", ""),
        ("a term over 60", "owner-a", long_term.as_str(), "m", ""),
        ("an empty meaning", "owner-a", "t", "", ""),
        ("a blank meaning", "owner-a", "t", "   ", ""),
        (
            "a meaning over 200",
            "owner-a",
            "t",
            long_meaning.as_str(),
            "",
        ),
        (
            "a question over 500",
            "owner-a",
            "t",
            "m",
            long_question.as_str(),
        ),
    ];
    for (what, owner, term, meaning, question) in cases {
        let result = sqlx::query(
            "INSERT INTO chat_term (owner, term, meaning, question) VALUES ($1, $2, $3, $4)",
        )
        .bind(owner)
        .bind(term)
        .bind(meaning)
        .bind(question)
        .execute(&pool)
        .await;
        assert!(result.is_err(), "{what} should be refused by a CHECK");
    }
    let (rows,): (i64,) = sqlx::query_as("SELECT count(*) FROM chat_term")
        .fetch_one(&pool)
        .await?;
    assert_eq!(rows, 0);
    Ok(())
}
