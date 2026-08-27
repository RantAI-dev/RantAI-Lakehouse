//! Integration tests for `lakehouse_store::governance` against a real
//! Postgres.
//!
//! # Why every test here is `#[ignore]`d
//!
//! Same reason as `tests/identity.rs`/`tests/schema.rs`: `#[sqlx::test]`
//! needs a live Postgres reachable via `DATABASE_URL`, and
//! `cargo test --workspace --locked` must stay green on a machine that has
//! none. Run them explicitly with:
//!
//! ```sh
//! docker compose up -d postgres
//! DATABASE_URL=postgres://lakehouse:lakehouse@localhost:5432/lakehouse \
//!   cargo test -p lakehouse-store -- --ignored
//! ```
//!
//! Every test below provisions a database with all migrations applied, so
//! the `0003_governance` seed fixtures are present.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_store::StoreError;
use lakehouse_store::governance::{
    CreateClassificationRuleInput, CreatePolicyInput, CreateQualityRuleInput,
    CreateResidencyRuleInput, create_classification_rule, create_policy, create_quality_rule,
    create_residency_rule, list_policies,
};
use sqlx::PgPool;

/// The seed lands the two `mock/governance.ts` policy fixtures, in fixture
/// order.
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn seed_populates_policies(pool: PgPool) -> sqlx::Result<()> {
    let policies = list_policies(&pool).await.unwrap();
    assert_eq!(policies.len(), 2);
    assert_eq!(policies[0].name, "tenant_row_filter_default");
    assert_eq!(policies[0].status, "ready");
    Ok(())
}

/// `activate: false` -> `"draft"`; `activate: true` -> `"ready"`; an absent
/// `owner` falls back to `"Current user"` — the exact defaults
/// `mock/governance.ts`'s `createPolicy` used.
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn create_policy_applies_mock_defaults(pool: PgPool) -> sqlx::Result<()> {
    let draft = create_policy(
        &pool,
        &CreatePolicyInput {
            name: "draft_policy".to_owned(),
            kind: "Row filter".to_owned(),
            subjects: "All".to_owned(),
            resources: "All".to_owned(),
            effect: "Deny".to_owned(),
            conditions: None,
            activate: false,
            owner: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(draft.status, "draft");
    assert_eq!(draft.owner, "Current user");
    assert_eq!(draft.version, 1);

    let ready = create_policy(
        &pool,
        &CreatePolicyInput {
            name: "ready_policy".to_owned(),
            kind: "Row filter".to_owned(),
            subjects: "All".to_owned(),
            resources: "All".to_owned(),
            effect: "Permit".to_owned(),
            conditions: Some("region = 'ID'".to_owned()),
            activate: true,
            owner: Some("Security".to_owned()),
        },
    )
    .await
    .unwrap();
    assert_eq!(ready.status, "ready");
    assert_eq!(ready.owner, "Security");
    Ok(())
}

/// Policy names are unique, matching `mock/governance.ts`'s fixture set
/// (every name distinct).
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn duplicate_policy_name_is_a_conflict(pool: PgPool) -> sqlx::Result<()> {
    let err = create_policy(
        &pool,
        &CreatePolicyInput {
            name: "tenant_row_filter_default".to_owned(),
            kind: "Row filter".to_owned(),
            subjects: "All".to_owned(),
            resources: "All".to_owned(),
            effect: "Deny".to_owned(),
            conditions: None,
            activate: false,
            owner: None,
        },
    )
    .await
    .expect_err("seeded name must collide");
    assert!(matches!(err, StoreError::Conflict), "{err:?}");
    Ok(())
}

/// A freshly authored quality rule starts `"warning"`/`now()`, matching
/// `mock/governance.ts`'s `createQualityRule` — it hasn't been run yet.
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn create_quality_rule_starts_warning(pool: PgPool) -> sqlx::Result<()> {
    let rule = create_quality_rule(
        &pool,
        &CreateQualityRuleInput {
            name: "new_rule".to_owned(),
            asset: "gold.revenue".to_owned(),
            dimension: "completeness".to_owned(),
            threshold: ">= 99%".to_owned(),
            severity: "high".to_owned(),
        },
    )
    .await
    .unwrap();
    assert_eq!(rule.last_status, "warning");
    assert!(rule.last_run_at.ends_with('Z'));
    Ok(())
}

/// A freshly authored classification rule starts `confidence: 1`,
/// `reviewStatus: "needs-review"` — matching `mock/governance.ts`'s
/// `createClassificationRule`. Optional fields round-trip when present and
/// absent alike.
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn create_classification_rule_starts_needs_review(pool: PgPool) -> sqlx::Result<()> {
    let with_column = create_classification_rule(
        &pool,
        &CreateClassificationRuleInput {
            asset: "core.customer.customer_360".to_owned(),
            column: Some("phone".to_owned()),
            classification: "confidential".to_owned(),
            masking_rule: Some("mask_phone".to_owned()),
        },
    )
    .await
    .unwrap();
    assert_eq!(with_column.confidence, 1);
    assert_eq!(with_column.review_status, "needs-review");
    assert_eq!(with_column.column.as_deref(), Some("phone"));

    let without_column = create_classification_rule(
        &pool,
        &CreateClassificationRuleInput {
            asset: "core.customer.customer_360".to_owned(),
            column: None,
            classification: "internal".to_owned(),
            masking_rule: None,
        },
    )
    .await
    .unwrap();
    assert!(without_column.column.is_none());
    assert!(without_column.masking_rule.is_none());
    Ok(())
}

/// A freshly authored residency rule starts `violations7d: 0`, matching
/// `mock/governance.ts`'s `createResidencyRule`.
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn create_residency_rule_starts_zero_violations(pool: PgPool) -> sqlx::Result<()> {
    let rule = create_residency_rule(
        &pool,
        &CreateResidencyRuleInput {
            tenant: "meridian-retail".to_owned(),
            classification: "restricted".to_owned(),
            approved_sites: vec!["Jakarta on-prem".to_owned()],
            cross_site_allowed: false,
            allowed_output: "Aggregates only".to_owned(),
        },
    )
    .await
    .unwrap();
    assert_eq!(rule.violations7d, 0);
    assert_eq!(rule.approved_sites, vec!["Jakarta on-prem".to_owned()]);
    Ok(())
}

/// The `0003_governance` seed is safe to apply twice, same convention as
/// `tests/identity.rs`'s `seed_is_idempotent_when_applied_twice`.
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn seed_is_idempotent_when_applied_twice(pool: PgPool) -> sqlx::Result<()> {
    let seed = include_str!("../../../migrations/0004_seed_governance.sql");
    sqlx::raw_sql(seed).execute(&pool).await?;

    assert_eq!(list_policies(&pool).await.unwrap().len(), 2);
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM quality_rule")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count.0, 2);
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM classification_rule")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count.0, 2);
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM residency_rule")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count.0, 2);
    Ok(())
}
