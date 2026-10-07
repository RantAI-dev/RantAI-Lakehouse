//! Integration tests for `semantic.rs` (migration `0059`): the store half of
//! the semantic layer, one description per table and column that the chat
//! reads and a person can correct.
//!
//! # Postgres backing
//!
//! `#[sqlx::test(migrations = "../../migrations")]` tests, backed by the
//! `lakehouse-test-support` testcontainer bootstrap, same arrangement as
//! `gold_publication.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_test_support as _;

use lakehouse_store::semantic::{self, SemanticInput};
use sqlx::PgPool;
use uuid::Uuid;

fn input(asset: &str, column: &str, description: &str) -> SemanticInput {
    SemanticInput {
        asset: asset.to_owned(),
        column_name: column.to_owned(),
        description: description.to_owned(),
        synonyms: Vec::new(),
        role: None,
    }
}

/// A draft over nothing is written, reported as written, and carries the
/// model's name and no author.
#[sqlx::test(migrations = "../../migrations")]
async fn semantic_insert_draft_writes_a_draft_with_the_models_name(
    pool: PgPool,
) -> sqlx::Result<()> {
    let mut draft = input("serving.orders", "amount", "Order total");
    draft.synonyms = vec!["total".to_owned(), "revenue".to_owned()];
    draft.role = Some("measure".to_owned());

    let written = semantic::insert_draft(&pool, &draft, "model-a")
        .await
        .expect("insert_draft should succeed");
    assert!(written, "a new draft must report that a row was written");

    let rows = semantic::list_for_asset(&pool, "serving.orders")
        .await
        .expect("list_for_asset should succeed");
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.column_name, "amount");
    assert_eq!(row.description, "Order total");
    assert_eq!(row.synonyms, ["total", "revenue"]);
    assert_eq!(row.role.as_deref(), Some("measure"));
    assert_eq!(row.status, "draft");
    assert_eq!(row.written_by, None);
    assert_eq!(row.model.as_deref(), Some("model-a"));
    Ok(())
}

/// Decision 3 of the plan: a person's text always wins. A draft over a
/// confirmed entry writes nothing and leaves every field as it was.
#[sqlx::test(migrations = "../../migrations")]
async fn semantic_insert_draft_over_a_confirmed_entry_writes_nothing(
    pool: PgPool,
) -> sqlx::Result<()> {
    let person = Uuid::new_v4();
    let mut confirmed = input("serving.orders", "amount", "Written by a person");
    confirmed.synonyms = vec!["total".to_owned()];
    confirmed.role = Some("measure".to_owned());
    semantic::confirm(&pool, &confirmed, person)
        .await
        .expect("confirm should succeed");

    let mut draft = input("serving.orders", "amount", "Written by the model");
    draft.synonyms = vec!["other".to_owned()];
    draft.role = Some("dimension".to_owned());
    let written = semantic::insert_draft(&pool, &draft, "model-a")
        .await
        .expect("insert_draft should succeed");
    assert!(!written, "a draft over an existing entry writes no row");

    let rows = semantic::list_for_asset(&pool, "serving.orders")
        .await
        .expect("list_for_asset should succeed");
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.description, "Written by a person");
    assert_eq!(row.synonyms, ["total"]);
    assert_eq!(row.role.as_deref(), Some("measure"));
    assert_eq!(row.status, "confirmed");
    assert_eq!(row.written_by, Some(person));
    assert_eq!(row.model, None);
    Ok(())
}

/// A second draft for the same cell does not replace the first one either:
/// a table is drafted once.
#[sqlx::test(migrations = "../../migrations")]
async fn semantic_insert_draft_over_a_draft_writes_nothing(pool: PgPool) -> sqlx::Result<()> {
    assert!(
        semantic::insert_draft(&pool, &input("silver.t", "", "first"), "model-a")
            .await
            .expect("first draft")
    );
    assert!(
        !semantic::insert_draft(&pool, &input("silver.t", "", "second"), "model-b")
            .await
            .expect("second draft")
    );
    let rows = semantic::list_for_asset(&pool, "silver.t").await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].description, "first");
    assert_eq!(rows[0].model.as_deref(), Some("model-a"));
    Ok(())
}

/// `confirm` over a draft replaces the text, marks it confirmed, records the
/// person and clears the model's name.
#[sqlx::test(migrations = "../../migrations")]
async fn semantic_confirm_over_a_draft_clears_the_model_and_sets_the_author(
    pool: PgPool,
) -> sqlx::Result<()> {
    semantic::insert_draft(
        &pool,
        &input("serving.orders", "amount", "Draft text"),
        "model-a",
    )
    .await
    .expect("insert_draft should succeed");

    let person = Uuid::new_v4();
    let mut corrected = input("serving.orders", "amount", "Corrected text");
    corrected.synonyms = vec!["total".to_owned()];
    corrected.role = Some("measure".to_owned());
    semantic::confirm(&pool, &corrected, person)
        .await
        .expect("confirm should succeed");

    let rows = semantic::list_for_asset(&pool, "serving.orders")
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "confirm replaces the draft in place");
    let row = &rows[0];
    assert_eq!(row.description, "Corrected text");
    assert_eq!(row.synonyms, ["total"]);
    assert_eq!(row.role.as_deref(), Some("measure"));
    assert_eq!(row.status, "confirmed");
    assert_eq!(row.written_by, Some(person));
    assert_eq!(row.model, None, "a person's text names no model");
    Ok(())
}

/// `confirm` with nothing to replace inserts the entry, and the table's own
/// entry (column `''`) sits beside its columns.
#[sqlx::test(migrations = "../../migrations")]
async fn semantic_confirm_inserts_when_there_is_no_entry_yet(pool: PgPool) -> sqlx::Result<()> {
    let person = Uuid::new_v4();
    semantic::confirm(&pool, &input("silver.t", "", "The table"), person)
        .await
        .expect("confirm should insert");
    let rows = semantic::list_for_asset(&pool, "silver.t").await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].column_name, "");
    assert_eq!(rows[0].status, "confirmed");
    Ok(())
}

/// `list_for_asset` answers one table only; `list_all` answers every table.
#[sqlx::test(migrations = "../../migrations")]
async fn semantic_list_for_asset_is_scoped_and_list_all_is_not(pool: PgPool) -> sqlx::Result<()> {
    for (asset, column) in [("serving.a", ""), ("serving.a", "x"), ("silver.b", "")] {
        semantic::insert_draft(&pool, &input(asset, column, "text"), "m")
            .await
            .unwrap();
    }
    let one = semantic::list_for_asset(&pool, "serving.a").await.unwrap();
    assert_eq!(one.len(), 2);
    assert!(one.iter().all(|row| row.asset == "serving.a"));
    let all = semantic::list_all(&pool).await.unwrap();
    assert_eq!(all.len(), 3);
    Ok(())
}

/// Insert one row around the store functions, to prove the table's own
/// rules: the store never builds an invalid row, so a `CHECK` is only
/// reachable by SQL.
async fn raw_insert(
    pool: &PgPool,
    column_name: &str,
    description: &str,
    synonyms: &[String],
    role: Option<&str>,
    status: &str,
    written_by: Option<Uuid>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO semantic_entry \
            (asset, column_name, description, synonyms, role, status, written_by) \
         VALUES ('serving.t', $1, $2, $3, $4, $5, $6)",
    )
    .bind(column_name)
    .bind(description)
    .bind(synonyms)
    .bind(role)
    .bind(status)
    .bind(written_by)
    .execute(pool)
    .await
    .map(|_| ())
}

fn assert_check_violation(result: Result<(), sqlx::Error>, what: &str) {
    let err = result.expect_err(what);
    assert!(
        err.as_database_error()
            .is_some_and(sqlx::error::DatabaseError::is_check_violation),
        "{what}: expected a CHECK violation, got {err:?}"
    );
}

/// The boundary values themselves are accepted, so the refusals below prove
/// the rule and not a rule that refuses everything.
#[sqlx::test(migrations = "../../migrations")]
async fn semantic_the_boundary_values_are_accepted(pool: PgPool) -> sqlx::Result<()> {
    let six: Vec<String> = (0..6).map(|i| format!("s{i}")).collect();
    raw_insert(
        &pool,
        "c",
        &"d".repeat(200),
        &six,
        Some("key"),
        "draft",
        None,
    )
    .await?;
    raw_insert(
        &pool,
        "",
        &"d".repeat(400),
        &["x".repeat(40)],
        None,
        "confirmed",
        Some(Uuid::new_v4()),
    )
    .await?;
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn semantic_a_seventh_synonym_is_refused(pool: PgPool) -> sqlx::Result<()> {
    let seven: Vec<String> = (0..7).map(|i| format!("s{i}")).collect();
    assert_check_violation(
        raw_insert(&pool, "c", "d", &seven, None, "draft", None).await,
        "seven synonyms",
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn semantic_a_41_character_synonym_is_refused(pool: PgPool) -> sqlx::Result<()> {
    assert_check_violation(
        raw_insert(&pool, "c", "d", &["x".repeat(41)], None, "draft", None).await,
        "a 41-character synonym",
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn semantic_an_empty_synonym_is_refused(pool: PgPool) -> sqlx::Result<()> {
    assert_check_violation(
        raw_insert(&pool, "c", "d", &[String::new()], None, "draft", None).await,
        "an empty synonym",
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn semantic_a_201_character_column_description_is_refused(pool: PgPool) -> sqlx::Result<()> {
    assert_check_violation(
        raw_insert(&pool, "c", &"d".repeat(201), &[], None, "draft", None).await,
        "a 201-character column description",
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn semantic_a_401_character_table_description_is_refused(pool: PgPool) -> sqlx::Result<()> {
    assert_check_violation(
        raw_insert(&pool, "", &"d".repeat(401), &[], None, "draft", None).await,
        "a 401-character table description",
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn semantic_an_unknown_role_is_refused(pool: PgPool) -> sqlx::Result<()> {
    assert_check_violation(
        raw_insert(&pool, "c", "d", &[], Some("metric"), "draft", None).await,
        "an unknown role",
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn semantic_a_role_on_the_table_itself_is_refused(pool: PgPool) -> sqlx::Result<()> {
    assert_check_violation(
        raw_insert(&pool, "", "d", &[], Some("key"), "draft", None).await,
        "a role on the table's own entry",
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn semantic_an_unknown_status_is_refused(pool: PgPool) -> sqlx::Result<()> {
    assert_check_violation(
        raw_insert(&pool, "c", "d", &[], None, "pending", None).await,
        "an unknown status",
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn semantic_a_draft_with_an_author_is_refused(pool: PgPool) -> sqlx::Result<()> {
    assert_check_violation(
        raw_insert(&pool, "c", "d", &[], None, "draft", Some(Uuid::new_v4())).await,
        "a draft with written_by set",
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn semantic_an_over_long_asset_or_column_name_is_refused(pool: PgPool) -> sqlx::Result<()> {
    assert_check_violation(
        raw_insert(&pool, &"c".repeat(201), "d", &[], None, "draft", None).await,
        "a 201-character column name",
    );
    let err = sqlx::query(
        "INSERT INTO semantic_entry (asset, description, status) VALUES ($1, 'd', 'draft')",
    )
    .bind("a".repeat(201))
    .execute(&pool)
    .await
    .map(|_| ());
    assert_check_violation(err, "a 201-character asset");
    Ok(())
}
