//! Integration tests for `lakehouse_store::connector_oauth` against a real
//! Postgres (`SRC-12` part C; same setup as `tests/connector_run.rs`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_test_support as _;

use lakehouse_store::StoreError;
use lakehouse_store::connector_oauth::{Pending, begin, consume};
use lakehouse_store::connectors::{
    CreateConnectorInput, CredentialKind, CredentialSource, CredentialSpec, create_connector,
    delete_connector,
};
use sqlx::PgPool;

const VERIFIER: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQ-_.~0123";

fn hash(n: u8) -> String {
    format!("{n:02x}").repeat(32)
}

async fn connector(pool: &PgPool, name: &str) -> String {
    let input = CreateConnectorInput {
        name: name.to_owned(),
        kind: "REST API".to_owned(),
        direction: "source".to_owned(),
        host: "api.example.com".to_owned(),
        credential: CredentialSpec {
            source: CredentialSource::Managed,
            primary: CredentialKind::SecretKey,
            secondary: Some(CredentialKind::Token),
        },
        environment: "staging".to_owned(),
        tenant: "Meridian Group".to_owned(),
        residency: String::new(),
        capabilities: vec![],
        owner: None,
    };
    create_connector(pool, &input).await.unwrap().0.id
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_state_works_once(pool: PgPool) {
    let id = connector(&pool, "oauth one").await;
    begin(&pool, &hash(1), &id, "u1", VERIFIER).await.unwrap();
    assert_eq!(
        consume(&pool, &hash(1)).await.unwrap(),
        Some(Pending {
            connector_id: id,
            principal_id: "u1".to_owned(),
            code_verifier: VERIFIER.to_owned(),
        })
    );
    assert_eq!(consume(&pool, &hash(1)).await.unwrap(), None, "a replay");
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_unknown_state_finds_nothing(pool: PgPool) {
    assert_eq!(consume(&pool, &hash(9)).await.unwrap(), None);
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_expired_state_finds_nothing_and_is_gone(pool: PgPool) {
    let id = connector(&pool, "oauth two").await;
    begin(&pool, &hash(1), &id, "u1", VERIFIER).await.unwrap();
    sqlx::query("UPDATE connector_oauth_state SET expires_at = now() - interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(consume(&pool, &hash(1)).await.unwrap(), None);
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM connector_oauth_state")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0, "consuming an expired state still deletes it");
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_state_lives_ten_minutes(pool: PgPool) {
    let id = connector(&pool, "oauth ttl").await;
    begin(&pool, &hash(1), &id, "u1", VERIFIER).await.unwrap();
    let seconds: f64 = sqlx::query_scalar(
        "SELECT extract(epoch FROM (expires_at - created_at))::float8 FROM connector_oauth_state",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!((seconds - 600.0).abs() < 5.0, "ttl was {seconds}s");
}

#[sqlx::test(migrations = "../../migrations")]
async fn only_the_hash_of_the_state_is_stored(pool: PgPool) {
    let id = connector(&pool, "oauth hash").await;
    begin(&pool, &hash(1), &id, "u1", VERIFIER).await.unwrap();
    let stored: String = sqlx::query_scalar("SELECT state_hash FROM connector_oauth_state")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, hash(1));
}

#[sqlx::test(migrations = "../../migrations")]
async fn starting_again_replaces_the_users_earlier_sign_in_only(pool: PgPool) {
    let id = connector(&pool, "oauth three").await;
    begin(&pool, &hash(1), &id, "u1", VERIFIER).await.unwrap();
    begin(&pool, &hash(2), &id, "u2", VERIFIER).await.unwrap();
    begin(&pool, &hash(3), &id, "u1", VERIFIER).await.unwrap();
    assert_eq!(consume(&pool, &hash(1)).await.unwrap(), None);
    assert!(consume(&pool, &hash(2)).await.unwrap().is_some());
    assert!(consume(&pool, &hash(3)).await.unwrap().is_some());
}

#[sqlx::test(migrations = "../../migrations")]
async fn deleting_the_connector_drops_its_pending_sign_ins(pool: PgPool) {
    let id = connector(&pool, "oauth four").await;
    begin(&pool, &hash(1), &id, "u1", VERIFIER).await.unwrap();
    assert!(delete_connector(&pool, &id).await.unwrap());
    assert_eq!(consume(&pool, &hash(1)).await.unwrap(), None);
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_unknown_connector_and_a_bad_value_are_refused(pool: PgPool) {
    assert!(matches!(
        begin(&pool, &hash(1), "nope", "u1", VERIFIER).await,
        Err(StoreError::NotFound)
    ));
    let id = connector(&pool, "oauth five").await;
    assert!(matches!(
        begin(&pool, "not-a-hash", &id, "u1", VERIFIER).await,
        Err(StoreError::Validation(_))
    ));
    assert!(matches!(
        begin(&pool, &hash(1), &id, "u1", "short").await,
        Err(StoreError::Validation(_))
    ));
}
