//! Integration tests for `lakehouse_store::knowledge` against a real
//! Postgres.
//!
//! # Why every test here is `#[ignore]`d
//!
//! Same reason as `tests/connectors.rs`: `#[sqlx::test]` needs a live
//! Postgres reachable via `DATABASE_URL`. Run explicitly with:
//!
//! ```sh
//! docker compose up -d postgres
//! DATABASE_URL=postgres://lakehouse:lakehouse@localhost:5432/lakehouse \
//!   cargo test -p lakehouse-store -- --ignored
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_store::StoreError;
use lakehouse_store::knowledge::{
    CreateSourceInput, CreateVectorJobInput, create_source, create_vector_job, list_sources,
    list_vector_jobs,
};
use sqlx::PgPool;

/// The seed lands the full `mock/knowledge.ts` fixture set.
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn seed_populates_knowledge_lists(pool: PgPool) -> sqlx::Result<()> {
    let sources = list_sources(&pool).await.unwrap();
    assert_eq!(sources.len(), 2);
    assert!(sources.iter().any(|s| s.id == "ks-supplier-policy"));

    let jobs = list_vector_jobs(&pool).await.unwrap();
    assert_eq!(jobs.len(), 2);
    assert!(jobs.iter().any(|j| j.id == "vj-policy"));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn create_source_starts_draft_and_indexing(pool: PgPool) -> sqlx::Result<()> {
    let input = CreateSourceInput {
        name: "new corpus".to_owned(),
        kind: "manual".to_owned(),
        embedding_model: "text-embed-3-small".to_owned(),
        classification: "internal".to_owned(),
        owner: None,
    };
    let created = create_source(&pool, &input).await.unwrap();
    assert_eq!(created.status, "draft");
    assert_eq!(created.index_status, "indexing");
    assert_eq!(created.chunk_count, 0);
    assert_eq!(created.owner, "Current user");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn duplicate_source_name_is_a_conflict(pool: PgPool) -> sqlx::Result<()> {
    let input = CreateSourceInput {
        name: "product-faq".to_owned(),
        kind: "web".to_owned(),
        embedding_model: "m".to_owned(),
        classification: "internal".to_owned(),
        owner: None,
    };
    let err = create_source(&pool, &input).await.unwrap_err();
    assert!(matches!(err, StoreError::Conflict));
    Ok(())
}

/// `createVectorJob` resolves `sourceId` by matching `source` against a
/// registered `knowledge_source.name`.
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn create_vector_job_resolves_source_id_from_source_name(pool: PgPool) -> sqlx::Result<()> {
    let input = CreateVectorJobInput {
        name: "new job".to_owned(),
        source: "product-faq".to_owned(),
        embedding_model: "text-embed-3-small".to_owned(),
        index_type: "HNSW".to_owned(),
        owner: None,
    };
    let created = create_vector_job(&pool, &input).await.unwrap();
    assert_eq!(created.status, "draft");
    assert_eq!(created.source_id.as_deref(), Some("ks-faq"));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn create_vector_job_leaves_source_id_none_for_unregistered_source(
    pool: PgPool,
) -> sqlx::Result<()> {
    let input = CreateVectorJobInput {
        name: "ad hoc job".to_owned(),
        source: "not a registered source".to_owned(),
        embedding_model: "m".to_owned(),
        index_type: "HNSW".to_owned(),
        owner: None,
    };
    let created = create_vector_job(&pool, &input).await.unwrap();
    assert!(created.source_id.is_none());
    Ok(())
}
