//! Integration tests for `lakehouse_store::schema_change` against a real
//! Postgres (`SRC-8` task 3): the baseline, the identity of a pending
//! change, approval, the connector pause and the cascade.
//!
//! Same `#[sqlx::test(migrations = "../../migrations")]` setup as
//! `tests/connectors.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_test_support as _;

use lakehouse_store::StoreError;
use lakehouse_store::connectors::{
    CreateConnectorInput, CredentialKind, CredentialSource, CredentialSpec, IngestSpecInput,
    UpdateConnectorInput, create_connector, delete_connector, get_connector, get_ingest_spec,
    list_ingestible_connectors, set_ingest_spec, update_connector,
};
use lakehouse_store::schema_change::{
    ApproveOutcome, ChangeKind, ChangeStatus, NewChange, ObservationTx, ObservationWrite,
    ObservedColumn, SCHEMA_CHANGE_PAUSE_REASON, TableCandidate, TableRefusal, approve_object,
    list_inactive_columns, list_pending, list_recent, record_table_additions,
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
    .into_approval()
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
            .into_approval()
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
        .into_approval()
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

// --- SRC-8 task 8: tables that appear at the source (decision D2) -------------

/// A connector with an ingest spec: a `PostgreSQL` dial and one table.
async fn connector_with_spec(pool: &PgPool) -> String {
    let id = connector(pool).await;
    set_ingest_spec(
        pool,
        &id,
        &IngestSpecInput {
            adapter: "sql".to_owned(),
            ingest_mode: "batch".to_owned(),
            dial: serde_json::json!({
                "driver": "postgres", "host": "db.internal", "port": 5432,
                "database": "shop", "user": "reader",
            }),
            source_objects: serde_json::json!([
                {"name": "public.orders", "target": "schema_change_test_orders", "loadMode": "append"}
            ]),
            schedule_cron: Some("0 2 * * *".to_owned()),
        },
    )
    .await
    .unwrap();
    id
}

fn candidate(name: &str, target: &str, refusal: Option<TableRefusal>) -> TableCandidate {
    TableCandidate {
        name: name.to_owned(),
        target: target.to_owned(),
        refusal,
    }
}

async fn objects(pool: &PgPool, id: &str) -> serde_json::Value {
    get_ingest_spec(pool, id)
        .await
        .unwrap()
        .unwrap()
        .source_objects
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_added_table_joins_the_connectors_tables_with_load_mode_replace_and_is_listed_as_applied(
    pool: PgPool,
) {
    let id = connector_with_spec(&pool).await;
    let before = get_ingest_spec(&pool, &id).await.unwrap().unwrap();

    let result = record_table_additions(
        &pool,
        &id,
        &[candidate(
            "public.customers",
            "schema_change_test_customers",
            None,
        )],
        Some("run-1"),
        OffsetDateTime::now_utc(),
    )
    .await
    .unwrap();

    assert_eq!(result.added, ["public.customers"]);
    assert!(result.not_added.is_empty());
    assert_eq!(
        objects(&pool, &id).await,
        serde_json::json!([
            {"name": "public.orders", "target": "schema_change_test_orders", "loadMode": "append"},
            {"name": "public.customers", "target": "schema_change_test_customers", "loadMode": "replace"},
        ])
    );
    // Everything else of the spec is written back as it was: SEC-14's
    // re-point rule saw the same target identity.
    let after = get_ingest_spec(&pool, &id).await.unwrap().unwrap();
    assert_eq!(after.dial, before.dial);
    assert_eq!(after.adapter, before.adapter);
    assert_eq!(after.schedule_cron, before.schedule_cron);

    let [change] = &result.changes[..] else {
        panic!("one change")
    };
    assert!(change.is_new);
    assert_eq!(change.change.kind, "table_added");
    assert_eq!(change.change.object_name, "public.customers");
    assert_eq!(change.change.status, "applied");
    assert_eq!(
        change.change.after_value.as_deref(),
        Some("schema_change_test_customers")
    );
    assert_eq!(change.change.run_id.as_deref(), Some("run-1"));
    assert!(!change.change.breaking);
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_second_identical_request_adds_nothing_and_writes_no_row(pool: PgPool) {
    let id = connector_with_spec(&pool).await;
    let ask = [candidate(
        "public.customers",
        "schema_change_test_customers",
        None,
    )];
    record_table_additions(&pool, &id, &ask, None, OffsetDateTime::now_utc())
        .await
        .unwrap();
    let stored = objects(&pool, &id).await;

    let again = record_table_additions(&pool, &id, &ask, None, OffsetDateTime::now_utc())
        .await
        .unwrap();

    assert!(again.added.is_empty() && again.not_added.is_empty() && again.changes.is_empty());
    assert_eq!(objects(&pool, &id).await, stored);
    let (rows,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM connector_schema_change WHERE connector_id = $1 AND kind = 'table_added'",
    )
    .bind(&id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(rows, 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_table_the_caller_refused_is_not_added_and_waits_with_the_fixed_reason(pool: PgPool) {
    let id = connector_with_spec(&pool).await;
    let stored = objects(&pool, &id).await;

    let result = record_table_additions(
        &pool,
        &id,
        &[candidate(
            "public.files",
            "taken_elsewhere",
            Some(TableRefusal::ReservedForUploads),
        )],
        None,
        OffsetDateTime::now_utc(),
    )
    .await
    .unwrap();

    assert!(result.added.is_empty());
    assert_eq!(
        result.not_added,
        [("public.files".to_owned(), TableRefusal::ReservedForUploads)]
    );
    assert_eq!(objects(&pool, &id).await, stored, "nothing was appended");
    let [change] = &result.changes[..] else {
        panic!("one change")
    };
    assert_eq!(change.change.status, "pending");
    assert_eq!(
        change.change.after_value.as_deref(),
        Some(TableRefusal::ReservedForUploads.reason())
    );
    // The same refusal, seen again, is the same pending row and is not new.
    let again = record_table_additions(
        &pool,
        &id,
        &[candidate(
            "public.files",
            "taken_elsewhere",
            Some(TableRefusal::ReservedForUploads),
        )],
        None,
        OffsetDateTime::now_utc(),
    )
    .await
    .unwrap();
    assert!(!again.changes[0].is_new);
    assert_eq!(list_pending(&pool, &id).await.unwrap().len(), 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_target_one_of_the_connectors_own_tables_already_uses_is_refused_under_the_lock(
    pool: PgPool,
) {
    let id = connector_with_spec(&pool).await;

    let result = record_table_additions(
        &pool,
        &id,
        &[
            // Same target as the stored `public.orders`.
            candidate("archive.orders", "schema_change_test_orders", None),
            // Two new tables that would land together: the first wins.
            candidate("public.a", "shared", None),
            candidate("public.b", "shared", None),
        ],
        None,
        OffsetDateTime::now_utc(),
    )
    .await
    .unwrap();

    assert_eq!(result.added, ["public.a"]);
    assert_eq!(
        result.not_added,
        [
            ("archive.orders".to_owned(), TableRefusal::TargetTaken),
            ("public.b".to_owned(), TableRefusal::TargetTaken),
        ]
    );
    let names: Vec<String> = objects(&pool, &id)
        .await
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(names, ["public.orders", "public.a"]);
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_connector_without_an_ingest_spec_cannot_take_tables_and_nothing_is_written(
    pool: PgPool,
) {
    let id = connector(&pool).await;
    let err = record_table_additions(
        &pool,
        &id,
        &[candidate("public.customers", "t", None)],
        None,
        OffsetDateTime::now_utc(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, StoreError::Validation(_)), "got {err:?}");
    let (rows,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM connector_schema_change WHERE connector_id = $1")
            .bind(&id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(rows, 0);
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_unknown_connector_is_not_found(pool: PgPool) {
    let err = record_table_additions(
        &pool,
        "conn-nope",
        &[candidate("public.customers", "t", None)],
        None,
        OffsetDateTime::now_utc(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, StoreError::NotFound), "got {err:?}");
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_ingestible_list_carries_the_schema_change_policy(pool: PgPool) {
    let id = connector_with_spec(&pool).await;
    sqlx::query("UPDATE connector SET schema_change_policy = 'apply_all' WHERE id = $1")
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
    let listed = list_ingestible_connectors(&pool).await.unwrap();
    let mine = listed.iter().find(|c| c.id == id).unwrap();
    assert_eq!(mine.schema_change_policy, "apply_all");
    assert_eq!(
        serde_json::to_value(mine).unwrap()["schemaChangePolicy"],
        "apply_all"
    );
}

fn type_change(column: &str, before: &str, after: &str) -> NewChange {
    NewChange {
        kind: ChangeKind::TypeChanged,
        column_name: column.to_owned(),
        before_value: Some(before.to_owned()),
        after_value: Some(after.to_owned()),
        breaking: true,
        status: ChangeStatus::Pending,
    }
}

async fn observe_changes(
    pool: &PgPool,
    id: &str,
    observed: &[ObservedColumn],
    changes: &[NewChange],
) {
    let mut tx = ObservationTx::begin(pool, id, "orders").await.unwrap();
    tx.record(&ObservationWrite {
        observed_columns: observed,
        observed_primary_key: &["id".to_owned()],
        changes,
        accept_observed: false,
        pause_connector: false,
        run_id: Some("run-1"),
        now: OffsetDateTime::now_utc(),
    })
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

/// `SRC-8 review BLOCKER 2` (decision 10): a waiting type change the column
/// cannot hold is not approvable, and it takes the table's other waiting
/// changes with it, because approval accepts the observed shape as a whole.
#[sqlx::test(migrations = "../../migrations")]
async fn a_type_change_the_column_cannot_hold_is_refused_and_nothing_is_approved(pool: PgPool) {
    let id = connector(&pool).await;
    baseline(&pool, &id, &[col("id", "integer"), col("qty", "integer")]).await;
    let observed = [
        col("id", "integer"),
        col("qty", "text"),
        col("note", "text"),
    ];
    let added = NewChange {
        kind: ChangeKind::ColumnAdded,
        column_name: "note".to_owned(),
        before_value: None,
        after_value: Some("text".to_owned()),
        breaking: false,
        status: ChangeStatus::Pending,
    };
    observe_changes(
        &pool,
        &id,
        &observed,
        &[type_change("qty", "integer", "text"), added],
    )
    .await;

    let pending = list_pending(&pool, &id).await.unwrap();
    assert_eq!(pending.len(), 2);
    assert!(
        pending.iter().all(|c| !c.can_approve),
        "the whole table is blocked: {pending:?}"
    );

    let outcome = approve_object(
        &pool,
        &id,
        "orders",
        Some("user-1"),
        OffsetDateTime::now_utc(),
    )
    .await
    .unwrap();
    assert_eq!(outcome, ApproveOutcome::CannotBeLoaded);
    assert_eq!(
        list_pending(&pool, &id).await.unwrap().len(),
        2,
        "not even the added column was approved"
    );
    assert!(list_recent(&pool, &id, 50).await.unwrap().is_empty());
}

/// The same refusal, then the exact sequence the console tells the person to
/// follow: integer to text is observed (waits), the source is put back, the
/// next observation shows no change and nothing is pending.
#[sqlx::test(migrations = "../../migrations")]
async fn the_table_leaves_the_waiting_state_when_the_source_is_put_back(pool: PgPool) {
    let id = connector(&pool).await;
    let original = [col("id", "integer"), col("qty", "integer")];
    baseline(&pool, &id, &original).await;
    observe_changes(
        &pool,
        &id,
        &[col("id", "integer"), col("qty", "text")],
        &[type_change("qty", "integer", "text")],
    )
    .await;
    assert_eq!(list_pending(&pool, &id).await.unwrap().len(), 1);

    observe_changes(&pool, &id, &original, &[]).await;

    assert!(list_pending(&pool, &id).await.unwrap().is_empty());
    assert_eq!(
        approve_object(&pool, &id, "orders", None, OffsetDateTime::now_utc())
            .await
            .unwrap(),
        ApproveOutcome::NothingWaits
    );
}

/// A narrowed type is breaking but loadable: it stays approvable.
#[sqlx::test(migrations = "../../migrations")]
async fn a_narrowed_type_change_is_still_approvable(pool: PgPool) {
    let id = connector(&pool).await;
    baseline(&pool, &id, &[col("id", "integer"), col("qty", "bigint")]).await;
    observe_changes(
        &pool,
        &id,
        &[col("id", "integer"), col("qty", "integer")],
        &[type_change("qty", "bigint", "integer")],
    )
    .await;
    assert!(list_pending(&pool, &id).await.unwrap()[0].can_approve);
    assert!(matches!(
        approve_object(&pool, &id, "orders", None, OffsetDateTime::now_utc())
            .await
            .unwrap(),
        ApproveOutcome::Approved(_)
    ));
}

/// `SRC-8 review SHOULD-FIX 3` (a), (b): a table that could not be added does
/// not count as waiting, so an approval elsewhere still lifts the pause.
#[sqlx::test(migrations = "../../migrations")]
async fn a_table_that_could_not_be_added_does_not_keep_the_connector_paused(pool: PgPool) {
    let id = connector_with_spec(&pool).await;
    baseline(&pool, &id, &[col("id", "integer"), col("note", "text")]).await;
    wait_for_removal(&pool, &id, &[col("id", "integer")], true).await;
    record_table_additions(
        &pool,
        &id,
        &[candidate(
            "public.files",
            "taken",
            Some(TableRefusal::TargetTaken),
        )],
        None,
        OffsetDateTime::now_utc(),
    )
    .await
    .unwrap();

    let approval = approve_object(&pool, &id, "orders", None, OffsetDateTime::now_utc())
        .await
        .unwrap()
        .into_approval()
        .unwrap();

    assert!(approval.pause_lifted, "the notice is not a waiting table");
    let pending = list_pending(&pool, &id).await.unwrap();
    assert_eq!(pending.len(), 1, "the notice itself is still listed");
    assert_eq!(pending[0].kind, "table_added");
}

/// `SRC-8 review SHOULD-FIX 3` (c), (d): approving the notice marks it seen
/// and adds nothing; the same refusal then makes no row and no alert, while a
/// different reason is a new row.
#[sqlx::test(migrations = "../../migrations")]
async fn a_dismissed_refusal_does_not_come_back_but_a_different_reason_does(pool: PgPool) {
    let id = connector_with_spec(&pool).await;
    let stored = objects(&pool, &id).await;
    let refuse = |reason| vec![candidate("public.files", "taken", Some(reason))];
    record_table_additions(
        &pool,
        &id,
        &refuse(TableRefusal::TargetTaken),
        None,
        OffsetDateTime::now_utc(),
    )
    .await
    .unwrap();

    let outcome = approve_object(
        &pool,
        &id,
        "public.files",
        Some("user-1"),
        OffsetDateTime::now_utc(),
    )
    .await
    .unwrap();
    let ApproveOutcome::Approved(approval) = outcome else {
        panic!("the notice is dismissed: {outcome:?}")
    };
    assert_eq!(approval.approved.len(), 1);
    assert_eq!(approval.approved[0].status, "approved");
    assert!(!approval.pause_lifted);
    assert_eq!(objects(&pool, &id).await, stored, "nothing was added");
    assert!(list_pending(&pool, &id).await.unwrap().is_empty());

    let again = record_table_additions(
        &pool,
        &id,
        &refuse(TableRefusal::TargetTaken),
        None,
        OffsetDateTime::now_utc(),
    )
    .await
    .unwrap();
    assert!(again.changes.is_empty(), "no new row, so no alert");
    assert_eq!(again.not_added.len(), 1, "still reported as not added");
    assert!(list_pending(&pool, &id).await.unwrap().is_empty());

    let other = record_table_additions(
        &pool,
        &id,
        &refuse(TableRefusal::ReservedForUploads),
        None,
        OffsetDateTime::now_utc(),
    )
    .await
    .unwrap();
    assert_eq!(other.changes.len(), 1);
    assert!(other.changes[0].is_new);
    assert_eq!(list_pending(&pool, &id).await.unwrap().len(), 1);
}

/// A table that was added (applied) is not affected by dismissals: it is
/// selected now, so a later request skips it and records nothing.
#[sqlx::test(migrations = "../../migrations")]
async fn an_added_table_is_never_recorded_again(pool: PgPool) {
    let id = connector_with_spec(&pool).await;
    let first = record_table_additions(
        &pool,
        &id,
        &[candidate(
            "public.invoices",
            "schema_change_test_invoices",
            None,
        )],
        None,
        OffsetDateTime::now_utc(),
    )
    .await
    .unwrap();
    assert_eq!(first.added.len(), 1);
    let second = record_table_additions(
        &pool,
        &id,
        &[candidate(
            "public.invoices",
            "schema_change_test_invoices",
            None,
        )],
        None,
        OffsetDateTime::now_utc(),
    )
    .await
    .unwrap();
    assert!(second.changes.is_empty() && second.added.is_empty());
    assert_eq!(list_recent(&pool, &id, 50).await.unwrap().len(), 1);
}
