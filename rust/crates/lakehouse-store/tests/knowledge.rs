//! Integration tests for `lakehouse_store::knowledge` against a real
//! Postgres.
//!
//! # Postgres backing
//!
//! These are `#[sqlx::test(migrations = "../../migrations")]` tests: each
//! one gets a freshly migrated, isolated database. The Postgres *server*
//! itself is started once per test binary by the `lakehouse-test-support`
//! dev-dependency, which spins up a `testcontainers`-managed Postgres and
//! points `DATABASE_URL` at it before any test runs — no manual
//! `docker compose up`, no external database required. Docker must be
//! reachable from the environment running `cargo test`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap actually runs for this test binary (an
// unreferenced dev-dependency's rlib member can otherwise be dropped
// by the linker before its ctor section is ever considered).
use lakehouse_test_support as _;

use lakehouse_store::StoreError;
use lakehouse_store::knowledge::{
    CreateSourceInput, CreateVectorJobInput, create_source, create_vector_job, list_sources,
    list_vector_jobs,
};
use sqlx::PgPool;

/// The seed lands the full `mock/knowledge.ts` fixture set.
#[sqlx::test(migrations = "../../migrations")]
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
