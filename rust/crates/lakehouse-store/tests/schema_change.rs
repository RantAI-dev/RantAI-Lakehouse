//! Integration tests for `lakehouse_store::schema_change` against a real
//! Postgres (`SRC-8` task 3): the baseline, the identity of a pending
//! change, approval, the connector pause and the cascade.
//!
//! Same `#[sqlx::test(migrations = "../../migrations")]` setup as
//! `tests/connectors.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_test_support as _;

use lakehouse_store::connectors::{
    CreateConnectorInput, CredentialKind, CredentialSource, CredentialSpec, UpdateConnectorInput,
    create_connector, delete_connector, get_connector, list_ingestible_connectors,
    update_connector,
};
use lakehouse_store::schema_change::{
    ChangeKind, ChangeStatus, NewChange, ObservationTx, ObservationWrite, ObservedColumn,
    SCHEMA_CHANGE_PAUSE_REASON, approve_object, list_inactive_columns, list_pending, list_recent,
};
use sqlx::PgPool;
use time::OffsetDateTime;

async fn connector(pool: &PgPool) -> String {
    let input = CreateConnectorInput {
        name: "schema change test".to_owned(),
        kind: "PostgreSQL".to_owned(),
        direction: "source".to_owned(),
        host: "db.example".to_owned(),
        credential: CredentialSpec {
            source: CredentialSource::Env,
            primary: CredentialKind::Password,
            secondary: None,
        },
        environment: "staging".to_owned(),
        tenant: "Meridian Group".to_owned(),
        residency: "in-region".to_owned(),
        capabilities: vec![],
        owner: None,
    };
    create_connector(pool, &input).await.unwrap().0.id
}

fn col(name: &str, type_name: &str) -> ObservedColumn {
    ObservedColumn {
        name: name.to_owned(),
        type_name: type_name.to_owned(),
        nullable: true,
    }
}

fn removed(column: &str, status: ChangeStatus) -> NewChange {
    NewChange {
        kind: ChangeKind::ColumnRemoved,
        column_name: column.to_owned(),
        before_value: Some("text".to_owned()),
        after_value: None,
        breaking: true,
        status,
    }
}

/// Write the first observation of `orders` as the baseline.
async fn baseline(pool: &PgPool, id: &str, columns: &[ObservedColumn]) {
    let mut tx = ObservationTx::begin(pool, id, "orders").await.unwrap();
    assert!(tx.accepted().await.unwrap().is_none());
    tx.record(&ObservationWrite {
        observed_columns: columns,
        observed_primary_key: &["id".to_owned()],
        changes: &[],
        accept_observed: true,
        pause_connector: false,
        run_id: None,
        now: OffsetDateTime::now_utc(),
    })
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

async fn wait_for_removal(pool: &PgPool, id: &str, observed: &[ObservedColumn], pause: bool) {
    let mut tx = ObservationTx::begin(pool, id, "orders").await.unwrap();
    tx.record(&ObservationWrite {
        observed_columns: observed,
        observed_primary_key: &["id".to_owned()],
        changes: &[removed("note", ChangeStatus::Pending)],
        accept_observed: false,
        pause_connector: pause,
        run_id: Some("run-1"),
        now: OffsetDateTime::now_utc(),
    })
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

/// A new connector starts on the default policy and is not paused.
#[sqlx::test(migrations = "../../migrations")]
async fn a_new_connector_has_the_default_policy_and_no_pause(pool: PgPool) -> sqlx::Result<()> {
    let id = connector(&pool).await;
    let detail = get_connector(&pool, &id).await.unwrap().unwrap();
    assert_eq!(detail.connector.schema_change_policy, "apply_non_breaking");
    assert_eq!(detail.connector.paused_reason, None);
    assert_eq!(detail.connector.paused_at, None);
    Ok(())
}

/// The policy is patchable, and the column refuses a fifth value.
#[sqlx::test(migrations = "../../migrations")]
async fn the_policy_can_be_set_and_an_unknown_one_is_refused(pool: PgPool) -> sqlx::Result<()> {
    let id = connector(&pool).await;
    let updated = update_connector(
        &pool,
        &id,
        &UpdateConnectorInput {
            schema_change_policy: Some("ask_first".to_owned()),
            ..UpdateConnectorInput::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(updated.schema_change_policy, "ask_first");
    let refused = update_connector(
        &pool,
        &id,
        &UpdateConnectorInput {
            schema_change_policy: Some("whatever".to_owned()),
            ..UpdateConnectorInput::default()
        },
    )
    .await;
    assert!(refused.is_err());
    Ok(())
}

/// A first observation is stored as the baseline and reads back.
#[sqlx::test(migrations = "../../migrations")]
async fn the_first_observation_becomes_the_baseline(pool: PgPool) -> sqlx::Result<()> {
    let id = connector(&pool).await;
    baseline(&pool, &id, &[col("id", "integer"), col("note", "text")]).await;
    let mut tx = ObservationTx::begin(&pool, &id, "orders").await.unwrap();
    let accepted = tx.accepted().await.unwrap().unwrap();
    assert_eq!(accepted.columns.len(), 2);
    assert_eq!(accepted.primary_key, vec!["id".to_owned()]);
    assert!(accepted.waiting_columns.is_none());
    Ok(())
}

/// A second identical baseline observation changes no row: the observation
/// time stays, and no change row exists.
#[sqlx::test(migrations = "../../migrations")]
async fn a_second_identical_observation_writes_nothing(pool: PgPool) -> sqlx::Result<()> {
    let id = connector(&pool).await;
    let columns = [col("id", "integer")];
    baseline(&pool, &id, &columns).await;
    let first: (OffsetDateTime,) =
        sqlx::query_as("SELECT observed_at FROM connector_source_schema WHERE connector_id = $1")
            .bind(&id)
            .fetch_one(&pool)
            .await?;
    baseline(&pool, &id, &columns).await;
    let second: (OffsetDateTime,) =
        sqlx::query_as("SELECT observed_at FROM connector_source_schema WHERE connector_id = $1")
            .bind(&id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(first, second);
    assert!(list_pending(&pool, &id).await.unwrap().is_empty());
    assert!(list_recent(&pool, &id, 50).await.unwrap().is_empty());
    Ok(())
}

/// A pending change keeps the baseline, stores the observed shape, and seen
/// again is the same row, reported as not new.
#[sqlx::test(migrations = "../../migrations")]
async fn a_pending_change_seen_twice_is_one_row_and_the_second_is_not_new(
    pool: PgPool,
) -> sqlx::Result<()> {
    let id = connector(&pool).await;
    baseline(&pool, &id, &[col("id", "integer"), col("note", "text")]).await;
    let observed = [col("id", "integer")];
    for expected_new in [true, false] {
        let mut tx = ObservationTx::begin(&pool, &id, "orders").await.unwrap();
        let recorded = tx
            .record(&ObservationWrite {
                observed_columns: &observed,
                observed_primary_key: &["id".to_owned()],
                changes: &[removed("note", ChangeStatus::Pending)],
                accept_observed: false,
                pause_connector: false,
                run_id: Some("run-1"),
                now: OffsetDateTime::now_utc(),
            })
            .await
            .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].is_new, expected_new);
    }
    assert_eq!(list_pending(&pool, &id).await.unwrap().len(), 1);
    let mut tx = ObservationTx::begin(&pool, &id, "orders").await.unwrap();
    let accepted = tx.accepted().await.unwrap().unwrap();
    assert_eq!(
        accepted.columns.len(),
        2,
        "a pending change does not move the baseline"
    );
    assert_eq!(accepted.waiting_columns.unwrap().len(), 1);
    Ok(())
}

/// A pending change the source stops showing is withdrawn.
#[sqlx::test(migrations = "../../migrations")]
async fn a_pending_change_that_no_longer_holds_is_withdrawn(pool: PgPool) -> sqlx::Result<()> {
    let id = connector(&pool).await;
    let full = [col("id", "integer"), col("note", "text")];
    baseline(&pool, &id, &full).await;
    wait_for_removal(&pool, &id, &[col("id", "integer")], false).await;
    assert_eq!(list_pending(&pool, &id).await.unwrap().len(), 1);
    let mut tx = ObservationTx::begin(&pool, &id, "orders").await.unwrap();
    tx.record(&ObservationWrite {
        observed_columns: &full,
        observed_primary_key: &["id".to_owned()],
        changes: &[],
        accept_observed: false,
        pause_connector: false,
        run_id: None,
        now: OffsetDateTime::now_utc(),
    })
    .await
    .unwrap();
    let accepted = tx.accepted().await.unwrap().unwrap();
    tx.commit().await.unwrap();
    assert!(list_pending(&pool, &id).await.unwrap().is_empty());
    assert!(accepted.waiting_columns.is_none());
    Ok(())
}

/// Approval accepts the observed shape, marks the removed column inactive,
/// lifts the schema-change pause and is `None` the second time.
#[sqlx::test(migrations = "../../migrations")]
async fn approval_moves_the_baseline_marks_inactive_and_lifts_the_pause(
    pool: PgPool,
) -> sqlx::Result<()> {
    let id = connector(&pool).await;
    baseline(&pool, &id, &[col("id", "integer"), col("note", "text")]).await;
    wait_for_removal(&pool, &id, &[col("id", "integer")], true).await;
    let paused = get_connector(&pool, &id).await.unwrap().unwrap().connector;
    assert_eq!(
        paused.paused_reason.as_deref(),
        Some(SCHEMA_CHANGE_PAUSE_REASON)
    );
    assert!(paused.paused_at.is_some());

    let approval = approve_object(
        &pool,
        &id,
        "orders",
        Some("user-1"),
        OffsetDateTime::now_utc(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(approval.approved.len(), 1);
    assert_eq!(approval.approved[0].status, "approved");
    assert!(approval.pause_lifted);

    let inactive = list_inactive_columns(&pool, &id).await.unwrap();
    assert_eq!(inactive.len(), 1);
    assert_eq!(inactive[0].column_name, "note");
    let mut tx = ObservationTx::begin(&pool, &id, "orders").await.unwrap();
    let accepted = tx.accepted().await.unwrap().unwrap();
    assert_eq!(accepted.columns, vec![col("id", "integer")]);
    assert!(accepted.waiting_columns.is_none());
    drop(tx);
    let after = get_connector(&pool, &id).await.unwrap().unwrap().connector;
    assert_eq!(after.paused_at, None);
    assert!(
        approve_object(&pool, &id, "orders", None, OffsetDateTime::now_utc())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(list_recent(&pool, &id, 50).await.unwrap().len(), 1);
    Ok(())
}

/// A pause set for another reason is not lifted by an approval.
#[sqlx::test(migrations = "../../migrations")]
async fn approval_leaves_a_pause_with_another_reason_alone(pool: PgPool) -> sqlx::Result<()> {
    let id = connector(&pool).await;
    baseline(&pool, &id, &[col("id", "integer"), col("note", "text")]).await;
    wait_for_removal(&pool, &id, &[col("id", "integer")], false).await;
    sqlx::query("UPDATE connector SET paused_reason = 'other', paused_at = now() WHERE id = $1")
        .bind(&id)
        .execute(&pool)
        .await?;
    let approval = approve_object(&pool, &id, "orders", None, OffsetDateTime::now_utc())
        .await
        .unwrap()
        .unwrap();
    assert!(!approval.pause_lifted);
    let after = get_connector(&pool, &id).await.unwrap().unwrap().connector;
    assert_eq!(after.paused_reason.as_deref(), Some("other"));
    Ok(())
}

/// A column that is accepted again stops being inactive.
#[sqlx::test(migrations = "../../migrations")]
async fn a_column_that_comes_back_is_no_longer_inactive(pool: PgPool) -> sqlx::Result<()> {
    let id = connector(&pool).await;
    baseline(&pool, &id, &[col("id", "integer"), col("note", "text")]).await;
    wait_for_removal(&pool, &id, &[col("id", "integer")], false).await;
    approve_object(&pool, &id, "orders", None, OffsetDateTime::now_utc())
        .await
        .unwrap();
    assert_eq!(list_inactive_columns(&pool, &id).await.unwrap().len(), 1);

    let back = [col("id", "integer"), col("note", "text")];
    let mut tx = ObservationTx::begin(&pool, &id, "orders").await.unwrap();
    tx.record(&ObservationWrite {
        observed_columns: &back,
        observed_primary_key: &["id".to_owned()],
        changes: &[NewChange {
            kind: ChangeKind::ColumnAdded,
            column_name: "note".to_owned(),
            before_value: None,
            after_value: Some("text".to_owned()),
            breaking: false,
            status: ChangeStatus::Applied,
        }],
        accept_observed: true,
        pause_connector: false,
        run_id: None,
        now: OffsetDateTime::now_utc(),
    })
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert!(list_inactive_columns(&pool, &id).await.unwrap().is_empty());
    Ok(())
}

/// The due list's source row carries `paused`, true only while paused.
#[sqlx::test(migrations = "../../migrations")]
async fn the_ingestible_list_says_which_connector_is_paused(pool: PgPool) -> sqlx::Result<()> {
    let id = connector(&pool).await;
    sqlx::query(
        "UPDATE connector SET adapter = 'sql', ingest_mode = 'batch', secret_ref = 'env:X' \
         WHERE id = $1",
    )
    .bind(&id)
    .execute(&pool)
    .await?;
    assert!(!list_ingestible_connectors(&pool).await.unwrap()[0].paused);
    sqlx::query("UPDATE connector SET paused_reason = 'r', paused_at = now() WHERE id = $1")
        .bind(&id)
        .execute(&pool)
        .await?;
    assert!(list_ingestible_connectors(&pool).await.unwrap()[0].paused);
    Ok(())
}

/// Deleting a connector removes its schema rows (ON DELETE CASCADE).
#[sqlx::test(migrations = "../../migrations")]
async fn deleting_a_connector_removes_its_schema_rows(pool: PgPool) -> sqlx::Result<()> {
    let id = connector(&pool).await;
    baseline(&pool, &id, &[col("id", "integer"), col("note", "text")]).await;
    wait_for_removal(&pool, &id, &[col("id", "integer")], false).await;
    approve_object(&pool, &id, "orders", None, OffsetDateTime::now_utc())
        .await
        .unwrap();
    assert!(delete_connector(&pool, &id).await.unwrap());
    for table in [
        "connector_source_schema",
        "connector_schema_change",
        "connector_inactive_column",
    ] {
        let (count,): (i64,) = sqlx::query_as(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(&pool)
            .await?;
        assert_eq!(count, 0, "{table}");
    }
    Ok(())
}
