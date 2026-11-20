//! Integration tests for `lakehouse_store::audit` against a real Postgres.
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
use lakehouse_store::audit::{
    ACTORS_MAX, AuditCursor, AuditFilter, AuditScope, NewAuditEvent, actors, count_matching,
    explain_search, get, insert, list, list_for_resources, search,
};
use serde_json::json;
use sqlx::PgPool;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

fn sample(action: &str) -> NewAuditEvent {
    NewAuditEvent {
        principal_id: Some("user-1".to_owned()),
        principal_kind: Some("user".to_owned()),
        actor_label: Some("Rina Wijaya".to_owned()),
        action: action.to_owned(),
        resource_kind: Some("connector".to_owned()),
        resource_id: Some("conn-1".to_owned()),
        args: Some(json!({ "foo": "bar" })),
        outcome: "executed".to_owned(),
        detail: None,
        run_id: None,
        approval_id: None,
        session_id: Some("sess-1".to_owned()),
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn insert_then_read_back(pool: PgPool) -> sqlx::Result<()> {
    let written = insert(&pool, sample("query_sql")).await.unwrap();
    assert!(written.id.starts_with("audit-"));
    assert_eq!(written.action, "query_sql");
    assert_eq!(written.args, json!({ "foo": "bar" }));
    assert_eq!(written.outcome, "executed");

    let events = list(&pool, AuditFilter::default(), &AuditScope::All)
        .await
        .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].id, written.id);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn list_orders_newest_first(pool: PgPool) -> sqlx::Result<()> {
    let first = insert(&pool, sample("first")).await.unwrap();
    let second = insert(&pool, sample("second")).await.unwrap();
    let third = insert(&pool, sample("third")).await.unwrap();

    let events = list(&pool, AuditFilter::default(), &AuditScope::All)
        .await
        .unwrap();
    assert_eq!(events.len(), 3);
    // All three rows are written with `at DEFAULT now()` from the same
    // statement-timestamp burst, so `id` order (insertion order) is used
    // as the tiebreak-independent check instead of asserting a strict
    // timestamp ordering that could tie.
    let ids: Vec<&str> = events.iter().map(|e| e.id.as_str()).collect();
    assert!(ids.contains(&third.id.as_str()));
    assert!(ids.contains(&second.id.as_str()));
    assert!(ids.contains(&first.id.as_str()));
    // Newest-first: the most recently inserted row's `at` must not be
    // earlier than an earlier one's.
    assert!(events[0].at >= events[1].at);
    assert!(events[1].at >= events[2].at);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn list_filters_by_resource(pool: PgPool) -> sqlx::Result<()> {
    let mut a = sample("a");
    a.resource_kind = Some("connector".to_owned());
    a.resource_id = Some("conn-1".to_owned());
    insert(&pool, a).await.unwrap();

    let mut b = sample("b");
    b.resource_kind = Some("alert_rule".to_owned());
    b.resource_id = Some("alert-9".to_owned());
    insert(&pool, b).await.unwrap();

    let filtered = list(
        &pool,
        AuditFilter {
            resource_kind: Some("connector".to_owned()),
            resource_id: Some("conn-1".to_owned()),
            ..AuditFilter::default()
        },
        &AuditScope::All,
    )
    .await
    .unwrap();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].action, "a");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn list_filters_by_principal(pool: PgPool) -> sqlx::Result<()> {
    let mut mine = sample("mine");
    mine.principal_id = Some("user-1".to_owned());
    insert(&pool, mine).await.unwrap();

    let mut other = sample("other");
    other.principal_id = Some("user-2".to_owned());
    insert(&pool, other).await.unwrap();

    let filtered = list(
        &pool,
        AuditFilter {
            principal_id: Some("user-2".to_owned()),
            ..AuditFilter::default()
        },
        &AuditScope::All,
    )
    .await
    .unwrap();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].action, "other");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn list_respects_limit(pool: PgPool) -> sqlx::Result<()> {
    for i in 0..5 {
        insert(&pool, sample(&format!("action-{i}"))).await.unwrap();
    }
    let limited = list(
        &pool,
        AuditFilter {
            limit: 2,
            ..AuditFilter::default()
        },
        &AuditScope::All,
    )
    .await
    .unwrap();
    assert_eq!(limited.len(), 2);
    Ok(())
}

/// `run_id` references `agent_run(id) ON DELETE SET NULL` — deleting the
/// referenced run must null out the audit row's `run_id`, not fail the
/// delete or orphan the audit row.
///
/// Uses a freshly-inserted `agent_run` row rather than a seeded fixture id:
/// migration `0026_drop_seeded_agent_runs.sql` (copilot-operations T3.1)
/// removes the `0018_seed_agents.sql` fixture rows (`run-col-01` /
/// `ap-01`), since they were fabricated demo history, not real runs — see
/// that migration's header comment.
#[sqlx::test(migrations = "../../migrations")]
async fn deleting_referenced_run_nulls_out_run_id(pool: PgPool) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO agent_run (id, employee_id, status, trigger, actor, steps) \
         VALUES ('run-audit-fixture', 'emp-inventory', 'running', 'test', 'test-actor', \
         '[]'::jsonb)",
    )
    .execute(&pool)
    .await
    .unwrap();

    let mut event = sample("run_tool");
    event.run_id = Some("run-audit-fixture".to_owned());
    let written = insert(&pool, event).await.unwrap();
    assert_eq!(written.run_id.as_deref(), Some("run-audit-fixture"));

    sqlx::query("DELETE FROM agent_run WHERE id = $1")
        .bind("run-audit-fixture")
        .execute(&pool)
        .await
        .unwrap();

    let events = list(&pool, AuditFilter::default(), &AuditScope::All)
        .await
        .unwrap();
    let row = events.iter().find(|e| e.id == written.id).unwrap();
    assert_eq!(row.run_id, None);
    Ok(())
}

/// Same `ON DELETE SET NULL` guarantee for `approval_id` ->
/// `approval_item(id)`. See [`deleting_referenced_run_nulls_out_run_id`]
/// for why this uses a freshly-inserted fixture rather than the
/// now-removed seeded `ap-01`.
#[sqlx::test(migrations = "../../migrations")]
async fn deleting_referenced_approval_nulls_out_approval_id(pool: PgPool) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO approval_item (id, employee_id, employee_name, action, status) \
         VALUES ('ap-audit-fixture', 'emp-inventory', 'inventory-copilot', 'test action', \
         'pending')",
    )
    .execute(&pool)
    .await
    .unwrap();

    let mut event = sample("decide_approval");
    event.approval_id = Some("ap-audit-fixture".to_owned());
    let written = insert(&pool, event).await.unwrap();
    assert_eq!(written.approval_id.as_deref(), Some("ap-audit-fixture"));

    sqlx::query("DELETE FROM approval_item WHERE id = $1")
        .bind("ap-audit-fixture")
        .execute(&pool)
        .await
        .unwrap();

    let events = list(&pool, AuditFilter::default(), &AuditScope::All)
        .await
        .unwrap();
    let row = events.iter().find(|e| e.id == written.id).unwrap();
    assert_eq!(row.approval_id, None);
    Ok(())
}

/// Asserts that a statement was refused by the append-only trigger of
/// `audit_event` (migration `0142_audit_event_append_only.sql`), not by some
/// other error such as a typo in the SQL.
fn assert_refused_as_append_only(result: Result<sqlx::postgres::PgQueryResult, sqlx::Error>) {
    let err = result.unwrap_err();
    let message = err
        .as_database_error()
        .map(|db| db.message().to_owned())
        .unwrap_or_default();
    assert!(
        message.contains("audit_event is append-only"),
        "expected the append-only trigger to refuse the statement, got: {err}"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn deleting_an_audit_event_is_refused(pool: PgPool) -> sqlx::Result<()> {
    let written = insert(&pool, sample("query_sql")).await.unwrap();

    assert_refused_as_append_only(
        sqlx::query("DELETE FROM audit_event WHERE id = $1")
            .bind(&written.id)
            .execute(&pool)
            .await,
    );

    let events = list(&pool, AuditFilter::default(), &AuditScope::All)
        .await
        .unwrap();
    assert_eq!(events.len(), 1);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn truncating_audit_event_is_refused(pool: PgPool) -> sqlx::Result<()> {
    insert(&pool, sample("query_sql")).await.unwrap();

    assert_refused_as_append_only(sqlx::query("TRUNCATE audit_event").execute(&pool).await);

    let events = list(&pool, AuditFilter::default(), &AuditScope::All)
        .await
        .unwrap();
    assert_eq!(events.len(), 1);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn updating_any_other_column_of_an_audit_event_is_refused(pool: PgPool) -> sqlx::Result<()> {
    let written = insert(&pool, sample("query_sql")).await.unwrap();

    for statement in [
        "UPDATE audit_event SET outcome = 'failed' WHERE id = $1",
        "UPDATE audit_event SET action = 'rewritten' WHERE id = $1",
        "UPDATE audit_event SET args = '{\"foo\": \"changed\"}'::jsonb WHERE id = $1",
        "UPDATE audit_event SET detail = 'edited afterwards' WHERE id = $1",
        "UPDATE audit_event SET actor_label = 'someone else' WHERE id = $1",
        "UPDATE audit_event SET at = now() + interval '1 day' WHERE id = $1",
    ] {
        assert_refused_as_append_only(
            sqlx::query(statement)
                .bind(&written.id)
                .execute(&pool)
                .await,
        );
    }

    // A change to a link column rides along with a forbidden change: the
    // whole statement is refused, the link is not nulled as a side effect.
    sqlx::query(
        "INSERT INTO agent_run (id, employee_id, status, trigger, actor, steps) \
         VALUES ('run-append-only', 'emp-inventory', 'running', 'test', 'test-actor', \
         '[]'::jsonb)",
    )
    .execute(&pool)
    .await
    .unwrap();
    let mut linked = sample("run_tool");
    linked.run_id = Some("run-append-only".to_owned());
    let linked = insert(&pool, linked).await.unwrap();
    assert_refused_as_append_only(
        sqlx::query("UPDATE audit_event SET run_id = NULL, outcome = 'failed' WHERE id = $1")
            .bind(&linked.id)
            .execute(&pool)
            .await,
    );

    let events = list(&pool, AuditFilter::default(), &AuditScope::All)
        .await
        .unwrap();
    let row = events.iter().find(|e| e.id == written.id).unwrap();
    assert_eq!(row, &written);
    let row = events.iter().find(|e| e.id == linked.id).unwrap();
    assert_eq!(row, &linked);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_audit_event_link_can_be_cleared_but_never_pointed_elsewhere(
    pool: PgPool,
) -> sqlx::Result<()> {
    for id in ["run-link-a", "run-link-b"] {
        sqlx::query(
            "INSERT INTO agent_run (id, employee_id, status, trigger, actor, steps) \
             VALUES ($1, 'emp-inventory', 'running', 'test', 'test-actor', '[]'::jsonb)",
        )
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    }
    let mut event = sample("run_tool");
    event.run_id = Some("run-link-a".to_owned());
    let written = insert(&pool, event).await.unwrap();

    assert_refused_as_append_only(
        sqlx::query("UPDATE audit_event SET run_id = 'run-link-b' WHERE id = $1")
            .bind(&written.id)
            .execute(&pool)
            .await,
    );
    // Attaching a link to a row that has none is a change too, not a clearing.
    let unlinked = insert(&pool, sample("query_sql")).await.unwrap();
    assert_refused_as_append_only(
        sqlx::query("UPDATE audit_event SET run_id = 'run-link-b' WHERE id = $1")
            .bind(&unlinked.id)
            .execute(&pool)
            .await,
    );

    sqlx::query("UPDATE audit_event SET run_id = NULL WHERE id = $1")
        .bind(&written.id)
        .execute(&pool)
        .await
        .unwrap();

    let events = list(&pool, AuditFilter::default(), &AuditScope::All)
        .await
        .unwrap();
    let row = events.iter().find(|e| e.id == written.id).unwrap();
    assert_eq!(row.run_id, None);
    assert_eq!(row.action, "run_tool");
    let row = events.iter().find(|e| e.id == unlinked.id).unwrap();
    assert_eq!(row.run_id, None);
    Ok(())
}

/// An unknown `run_id` is a foreign-key violation, not a silent insert.
#[sqlx::test(migrations = "../../migrations")]
async fn insert_with_unknown_run_id_is_a_foreign_key_violation(pool: PgPool) -> sqlx::Result<()> {
    let mut event = sample("nope");
    event.run_id = Some("run-does-not-exist".to_owned());
    let err = insert(&pool, event).await.unwrap_err();
    assert!(matches!(err, StoreError::ForeignKeyViolation));
    Ok(())
}

/// One asset is recorded under several ids; its history is every event
/// under any of them, of that resource kind only, newest first.
#[sqlx::test(migrations = "../../migrations")]
async fn list_for_resources_gathers_one_kind_across_several_ids(pool: PgPool) -> sqlx::Result<()> {
    let event = |kind: &str, id: &str, action: &str| NewAuditEvent {
        action: action.to_owned(),
        resource_kind: Some(kind.to_owned()),
        resource_id: Some(id.to_owned()),
        outcome: "executed".to_owned(),
        ..Default::default()
    };
    insert(&pool, event("catalog", "demo-orders", "catalog.annotate"))
        .await
        .unwrap();
    insert(
        &pool,
        event("catalog", "bronze.demo_orders", "quality.rule_create"),
    )
    .await
    .unwrap();
    insert(&pool, event("catalog", "silver.other", "catalog.annotate"))
        .await
        .unwrap();
    insert(&pool, event("connector", "demo-orders", "connector.test"))
        .await
        .unwrap();

    let ids = ["demo-orders".to_owned(), "bronze.demo_orders".to_owned()];
    let found = list_for_resources(&pool, "catalog", &ids, 50, &AuditScope::All)
        .await
        .unwrap();
    let actions: Vec<&str> = found.iter().map(|e| e.action.as_str()).collect();
    assert_eq!(actions, vec!["quality.rule_create", "catalog.annotate"]);

    assert_eq!(
        list_for_resources(&pool, "catalog", &[], 50, &AuditScope::All)
            .await
            .unwrap(),
        [] as [lakehouse_store::audit::AuditEvent; 0]
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Search: filters, keyset paging, count, one event by id (GOV-1 part B).
// ---------------------------------------------------------------------------

fn ts(rfc3339: &str) -> OffsetDateTime {
    OffsetDateTime::parse(rfc3339, &Rfc3339).unwrap()
}

/// One row to seed with an explicit id and `at`. `insert` generates both, and
/// a search test needs to pin them: equal timestamps, microsecond gaps, a
/// boundary exactly on a filter's edge.
struct Seed<'a> {
    id: &'a str,
    at: &'a str,
    principal: Option<&'a str>,
    action: &'a str,
    outcome: &'a str,
    resource: Option<(&'a str, &'a str)>,
}

impl<'a> Seed<'a> {
    fn new(id: &'a str, at: &'a str, action: &'a str) -> Self {
        Self {
            id,
            at,
            principal: None,
            action,
            outcome: "executed",
            resource: None,
        }
    }
}

async fn seed(pool: &PgPool, rows: &[Seed<'_>]) {
    for row in rows {
        sqlx::query(
            "INSERT INTO audit_event (id, at, principal_id, action, outcome, resource_kind, \
             resource_id) VALUES ($1, $2::timestamptz, $3, $4, $5, $6, $7)",
        )
        .bind(row.id)
        .bind(row.at)
        .bind(row.principal)
        .bind(row.action)
        .bind(row.outcome)
        .bind(row.resource.map(|(kind, _)| kind))
        .bind(row.resource.map(|(_, id)| id))
        .execute(pool)
        .await
        .unwrap();
    }
}

fn ids(page: &lakehouse_store::audit::AuditPage) -> Vec<&str> {
    page.events.iter().map(|e| e.id.as_str()).collect()
}

#[sqlx::test(migrations = "../../migrations")]
async fn search_by_actor_returns_only_that_persons_events(pool: PgPool) -> sqlx::Result<()> {
    let mut a = Seed::new("e-a", "2026-03-01 10:00:00+00", "dashboard.update");
    a.principal = Some("user-1");
    let mut b = Seed::new("e-b", "2026-03-01 11:00:00+00", "dashboard.update");
    b.principal = Some("user-2");
    seed(
        &pool,
        &[a, b, Seed::new("e-c", "2026-03-01 12:00:00+00", "x")],
    )
    .await;

    let page = search(
        &pool,
        &AuditFilter {
            principal_id: Some("user-2".to_owned()),
            ..AuditFilter::default()
        },
        &AuditScope::All,
    )
    .await
    .unwrap();
    assert_eq!(ids(&page), ["e-b"]);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn search_by_action_without_a_trailing_dot_matches_that_action_exactly(
    pool: PgPool,
) -> sqlx::Result<()> {
    seed(
        &pool,
        &[
            Seed::new("e-1", "2026-03-01 10:00:00+00", "dashboard.update"),
            Seed::new("e-2", "2026-03-01 11:00:00+00", "dashboard.update.bulk"),
            Seed::new("e-3", "2026-03-01 12:00:00+00", "dashboard"),
            Seed::new("e-4", "2026-03-01 13:00:00+00", "query.update"),
        ],
    )
    .await;

    let page = search(
        &pool,
        &AuditFilter {
            action: Some("dashboard.update".to_owned()),
            ..AuditFilter::default()
        },
        &AuditScope::All,
    )
    .await
    .unwrap();
    assert_eq!(ids(&page), ["e-1"]);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn search_by_action_ending_in_a_dot_matches_every_action_with_that_prefix(
    pool: PgPool,
) -> sqlx::Result<()> {
    seed(
        &pool,
        &[
            Seed::new("e-1", "2026-03-01 10:00:00+00", "dashboard.update"),
            Seed::new("e-2", "2026-03-01 11:00:00+00", "dashboard.share.create"),
            Seed::new("e-3", "2026-03-01 12:00:00+00", "dashboardx.update"),
            Seed::new("e-4", "2026-03-01 13:00:00+00", "query.update"),
        ],
    )
    .await;

    let page = search(
        &pool,
        &AuditFilter {
            action: Some("dashboard.".to_owned()),
            ..AuditFilter::default()
        },
        &AuditScope::All,
    )
    .await
    .unwrap();
    assert_eq!(ids(&page), ["e-2", "e-1"]);
    Ok(())
}

/// `_` and `%` are `LIKE` wildcards. A prefix typed by a person must match
/// them literally, or `a_b.` would also find `axb.create`.
#[sqlx::test(migrations = "../../migrations")]
async fn search_by_action_prefix_treats_like_wildcards_as_plain_characters(
    pool: PgPool,
) -> sqlx::Result<()> {
    seed(
        &pool,
        &[
            Seed::new("e-1", "2026-03-01 10:00:00+00", "a_b.create"),
            Seed::new("e-2", "2026-03-01 11:00:00+00", "axb.create"),
            Seed::new("e-3", "2026-03-01 12:00:00+00", "100%.done"),
            Seed::new("e-4", "2026-03-01 13:00:00+00", "100x.done"),
            Seed::new("e-5", "2026-03-01 14:00:00+00", "back\\slash.run"),
        ],
    )
    .await;

    for (prefix, expected) in [
        ("a_b.", vec!["e-1"]),
        ("100%.", vec!["e-3"]),
        ("back\\slash.", vec!["e-5"]),
        ("%.", vec![]),
    ] {
        let page = search(
            &pool,
            &AuditFilter {
                action: Some(prefix.to_owned()),
                ..AuditFilter::default()
            },
            &AuditScope::All,
        )
        .await
        .unwrap();
        assert_eq!(ids(&page), expected, "prefix {prefix:?}");
    }
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn search_by_outcome_returns_only_that_outcome(pool: PgPool) -> sqlx::Result<()> {
    let mut refused = Seed::new("e-1", "2026-03-01 10:00:00+00", "role.update");
    refused.outcome = "refused";
    seed(
        &pool,
        &[
            refused,
            Seed::new("e-2", "2026-03-01 11:00:00+00", "role.update"),
        ],
    )
    .await;

    let page = search(
        &pool,
        &AuditFilter {
            outcome: Some("refused".to_owned()),
            ..AuditFilter::default()
        },
        &AuditScope::All,
    )
    .await
    .unwrap();
    assert_eq!(ids(&page), ["e-1"]);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn search_by_resource_kind_and_id_returns_events_about_that_object(
    pool: PgPool,
) -> sqlx::Result<()> {
    let mut a = Seed::new("e-1", "2026-03-01 10:00:00+00", "dashboard.update");
    a.resource = Some(("dashboard", "d-1"));
    let mut b = Seed::new("e-2", "2026-03-01 11:00:00+00", "dashboard.update");
    b.resource = Some(("dashboard", "d-2"));
    let mut c = Seed::new("e-3", "2026-03-01 12:00:00+00", "query.update");
    c.resource = Some(("query", "d-1"));
    seed(&pool, &[a, b, c]).await;

    let by_kind = search(
        &pool,
        &AuditFilter {
            resource_kind: Some("dashboard".to_owned()),
            ..AuditFilter::default()
        },
        &AuditScope::All,
    )
    .await
    .unwrap();
    assert_eq!(ids(&by_kind), ["e-2", "e-1"]);

    let by_both = search(
        &pool,
        &AuditFilter {
            resource_kind: Some("dashboard".to_owned()),
            resource_id: Some("d-1".to_owned()),
            ..AuditFilter::default()
        },
        &AuditScope::All,
    )
    .await
    .unwrap();
    assert_eq!(ids(&by_both), ["e-1"]);
    Ok(())
}

/// `since` keeps the row that sits exactly on it; `until` drops the row that
/// sits exactly on it. A date range in the console reads "from this
/// instant, up to but not including that one", so two adjacent ranges never
/// show the same event twice nor skip one.
#[sqlx::test(migrations = "../../migrations")]
async fn search_since_is_inclusive_and_until_is_exclusive(pool: PgPool) -> sqlx::Result<()> {
    seed(
        &pool,
        &[
            Seed::new("e-1", "2026-03-01 09:59:59.999999+00", "a.b"),
            Seed::new("e-2", "2026-03-01 10:00:00+00", "a.b"),
            Seed::new("e-3", "2026-03-01 11:00:00+00", "a.b"),
            Seed::new("e-4", "2026-03-01 12:00:00+00", "a.b"),
        ],
    )
    .await;

    let page = search(
        &pool,
        &AuditFilter {
            since: Some(ts("2026-03-01T10:00:00Z")),
            until: Some(ts("2026-03-01T12:00:00Z")),
            ..AuditFilter::default()
        },
        &AuditScope::All,
    )
    .await
    .unwrap();
    assert_eq!(ids(&page), ["e-3", "e-2"]);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn search_combines_every_filter_with_and(pool: PgPool) -> sqlx::Result<()> {
    let mut hit = Seed::new("e-hit", "2026-03-01 10:30:00+00", "dashboard.update");
    hit.principal = Some("user-1");
    hit.outcome = "refused";
    hit.resource = Some(("dashboard", "d-1"));

    // Each of these misses on exactly one filter.
    let mut wrong_actor = Seed::new("e-1", "2026-03-01 10:30:00+00", "dashboard.update");
    wrong_actor.principal = Some("user-2");
    wrong_actor.outcome = "refused";
    wrong_actor.resource = Some(("dashboard", "d-1"));
    let mut wrong_action = Seed::new("e-2", "2026-03-01 10:30:00+00", "query.update");
    wrong_action.principal = Some("user-1");
    wrong_action.outcome = "refused";
    wrong_action.resource = Some(("dashboard", "d-1"));
    let mut wrong_outcome = Seed::new("e-3", "2026-03-01 10:30:00+00", "dashboard.update");
    wrong_outcome.principal = Some("user-1");
    wrong_outcome.resource = Some(("dashboard", "d-1"));
    let mut wrong_resource = Seed::new("e-4", "2026-03-01 10:30:00+00", "dashboard.update");
    wrong_resource.principal = Some("user-1");
    wrong_resource.outcome = "refused";
    wrong_resource.resource = Some(("dashboard", "d-2"));
    let mut too_old = Seed::new("e-5", "2026-03-01 09:00:00+00", "dashboard.update");
    too_old.principal = Some("user-1");
    too_old.outcome = "refused";
    too_old.resource = Some(("dashboard", "d-1"));
    let mut too_new = Seed::new("e-6", "2026-03-01 11:00:00+00", "dashboard.update");
    too_new.principal = Some("user-1");
    too_new.outcome = "refused";
    too_new.resource = Some(("dashboard", "d-1"));
    seed(
        &pool,
        &[
            hit,
            wrong_actor,
            wrong_action,
            wrong_outcome,
            wrong_resource,
            too_old,
            too_new,
        ],
    )
    .await;

    let page = search(
        &pool,
        &AuditFilter {
            principal_id: Some("user-1".to_owned()),
            action: Some("dashboard.".to_owned()),
            outcome: Some("refused".to_owned()),
            resource_kind: Some("dashboard".to_owned()),
            resource_id: Some("d-1".to_owned()),
            since: Some(ts("2026-03-01T10:00:00Z")),
            until: Some(ts("2026-03-01T11:00:00Z")),
            ..AuditFilter::default()
        },
        &AuditScope::All,
    )
    .await
    .unwrap();
    assert_eq!(ids(&page), ["e-hit"]);
    Ok(())
}

/// Seven rows. `e-c`, `e-b`, `e-a` share one `at`. `e-d` and `e-e` share
/// another that is 100 microseconds later: the same millisecond, so a cursor
/// built from the JSON row's millisecond `at` would skip them. Newest first,
/// ties broken by id descending.
async fn seed_ties(pool: &PgPool) -> Vec<&'static str> {
    seed(
        pool,
        &[
            Seed::new("e-a", "2026-03-01 10:00:00.000100+00", "a.b"),
            Seed::new("e-b", "2026-03-01 10:00:00.000100+00", "a.b"),
            Seed::new("e-c", "2026-03-01 10:00:00.000100+00", "a.b"),
            Seed::new("e-d", "2026-03-01 10:00:00.000200+00", "a.b"),
            Seed::new("e-e", "2026-03-01 10:00:00.000200+00", "a.b"),
            Seed::new("e-f", "2026-03-01 10:00:01+00", "a.b"),
            Seed::new("e-g", "2026-03-01 09:59:59+00", "a.b"),
        ],
    )
    .await;
    vec!["e-f", "e-e", "e-d", "e-c", "e-b", "e-a", "e-g"]
}

#[sqlx::test(migrations = "../../migrations")]
async fn paging_through_rows_that_share_a_timestamp_shows_each_row_once_in_order(
    pool: PgPool,
) -> sqlx::Result<()> {
    let expected = seed_ties(&pool).await;

    for page_size in [1_i64, 2, 3, 6, 7] {
        let mut seen: Vec<String> = Vec::new();
        let mut cursor: Option<AuditCursor> = None;
        let mut pages = 0;
        loop {
            let page = search(
                &pool,
                &AuditFilter {
                    limit: page_size,
                    cursor: cursor.clone(),
                    ..AuditFilter::default()
                },
                &AuditScope::All,
            )
            .await
            .unwrap();
            assert!(i64::try_from(page.events.len()).unwrap() <= page_size);
            seen.extend(page.events.iter().map(|e| e.id.clone()));
            pages += 1;
            assert!(pages <= 10, "paging does not end");
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        assert_eq!(seen, expected, "page size {page_size}");
    }
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_page_that_ends_exactly_at_the_last_row_has_no_next_cursor(
    pool: PgPool,
) -> sqlx::Result<()> {
    seed_ties(&pool).await;

    let exact = search(
        &pool,
        &AuditFilter {
            limit: 7,
            ..AuditFilter::default()
        },
        &AuditScope::All,
    )
    .await
    .unwrap();
    assert_eq!(exact.events.len(), 7);
    assert!(exact.next_cursor.is_none());

    let short = search(
        &pool,
        &AuditFilter {
            limit: 6,
            ..AuditFilter::default()
        },
        &AuditScope::All,
    )
    .await
    .unwrap();
    assert_eq!(short.events.len(), 6);
    assert!(short.next_cursor.is_some());
    Ok(())
}

#[test]
fn a_cursor_survives_encode_and_decode() {
    let cursor = AuditCursor::decode(
        &AuditCursor::from_parts(1_772_359_200_000_100, "audit-x:y.z").encode(),
    )
    .unwrap();
    assert_eq!(
        cursor,
        AuditCursor::from_parts(1_772_359_200_000_100, "audit-x:y.z")
    );
    // Negative microseconds (before 1970) round trip as well.
    let before = AuditCursor::from_parts(-5, "e-1");
    assert_eq!(AuditCursor::decode(&before.encode()).unwrap(), before);
    // The text is url-safe: it goes in a query string as it is.
    assert!(
        cursor
            .encode()
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    );
}

#[test]
fn a_malformed_cursor_is_rejected_without_a_panic() {
    use base64::Engine as _;
    let b64 = |text: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(text);
    for bad in [
        String::new(),
        "!!!not base64!!!".to_owned(),
        "a b".to_owned(),
        // Valid base64 whose text is not `<microseconds>:<id>`.
        b64(b"no-colon-here"),
        b64(b"abc:e-1"),
        b64(b":e-1"),
        b64(b"1772359200000100:"),
        b64(&[0xff, 0xfe, 0x3a, 0x31]),
        // A microsecond count no timestamp can hold.
        b64(format!("{}:e-1", i64::MAX).as_bytes()),
        b64(format!("{}:e-1", i64::MIN).as_bytes()),
        // GOV-1 review R-1: `time` takes these but Postgres `timestamptz`
        // refuses them (before 4713 BC), so the read would answer 500.
        b64(b"-300000000000000000:e-1"),
        // The first microsecond before year 1 is outside the range the
        // decoder accepts (years 1 to 9999).
        b64(b"-62135596800000001:e-1"),
        // The first microsecond after year 9999.
        b64(b"253402300800000000:e-1"),
        // Postgres TEXT refuses a NUL byte, so a cursor id with one never
        // reaches the database.
        b64(b"1772359200000100:e\x00-1"),
    ] {
        assert!(AuditCursor::decode(&bad).is_err(), "accepted {bad:?}");
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn count_matching_ignores_the_limit_and_the_cursor(pool: PgPool) -> sqlx::Result<()> {
    seed_ties(&pool).await;
    let first = search(
        &pool,
        &AuditFilter {
            limit: 2,
            ..AuditFilter::default()
        },
        &AuditScope::All,
    )
    .await
    .unwrap();

    let all = count_matching(
        &pool,
        &AuditFilter {
            limit: 2,
            cursor: first.next_cursor.clone(),
            ..AuditFilter::default()
        },
        &AuditScope::All,
    )
    .await
    .unwrap();
    assert_eq!(all, 7);

    // It still honours the narrowing filters.
    let narrowed = count_matching(
        &pool,
        &AuditFilter {
            since: Some(ts("2026-03-01T10:00:00Z")),
            limit: 1,
            ..AuditFilter::default()
        },
        &AuditScope::All,
    )
    .await
    .unwrap();
    assert_eq!(narrowed, 6);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn get_returns_one_event_and_none_for_an_unknown_id(pool: PgPool) -> sqlx::Result<()> {
    let written = insert(&pool, sample("query_sql")).await.unwrap();

    let found = get(&pool, &written.id, &AuditScope::All).await.unwrap();
    assert_eq!(found, Some(written));
    assert_eq!(
        get(&pool, "audit-no-such-id", &AuditScope::All)
            .await
            .unwrap(),
        None
    );
    Ok(())
}

/// 600 rows, so both the default (100) and the cap (500) are visible.
async fn seed_many(pool: &PgPool, rows: i32) {
    sqlx::query(
        "INSERT INTO audit_event (id, at, action, outcome) \
         SELECT 'bulk-' || g, timestamptz '2026-01-01 00:00:00+00' + g * interval '1 second', \
         'a.b', 'executed' FROM generate_series(1, $1) g",
    )
    .bind(rows)
    .execute(pool)
    .await
    .unwrap();
}

#[sqlx::test(migrations = "../../migrations")]
async fn search_uses_a_default_limit_of_100_and_caps_the_limit_at_500(
    pool: PgPool,
) -> sqlx::Result<()> {
    seed_many(&pool, 600).await;

    for unset in [0_i64, -3] {
        let page = search(
            &pool,
            &AuditFilter {
                limit: unset,
                ..AuditFilter::default()
            },
            &AuditScope::All,
        )
        .await
        .unwrap();
        assert_eq!(page.events.len(), 100, "limit {unset}");
        assert!(page.next_cursor.is_some());
    }

    let capped = search(
        &pool,
        &AuditFilter {
            limit: 10_000,
            ..AuditFilter::default()
        },
        &AuditScope::All,
    )
    .await
    .unwrap();
    assert_eq!(capped.events.len(), 500);
    assert!(capped.next_cursor.is_some());

    let exact = search(
        &pool,
        &AuditFilter {
            limit: 25,
            ..AuditFilter::default()
        },
        &AuditScope::All,
    )
    .await
    .unwrap();
    assert_eq!(exact.events.len(), 25);
    Ok(())
}

// ---------------------------------------------------------------------------
// Indexes: which index serves each search shape on a table of realistic size.
// ---------------------------------------------------------------------------

/// 50,000 rows: about 50 people, 40 actions under 8 prefixes, the 8 outcomes
/// skewed toward `executed`, `at` spread over a year, 2,000 resources. Columns
/// come from a hash of the row number so that no column follows another (a
/// modulus of one number would tie principal, action and outcome together and
/// flatter an index). Insert only: migration 0142 refuses UPDATE and DELETE.
async fn seed_realistic_table(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO audit_event (id, at, principal_id, principal_kind, action, resource_kind, \
         resource_id, outcome) \
         SELECT 'seed-' || g, \
                timestamptz '2026-01-01 00:00:00+00' + (g * 631) * interval '1 second', \
                'user-' || (abs(hashtext('p' || g)::bigint) % 50), \
                'user', \
                (ARRAY['dashboard','query','pipeline','connector','alert','identity','policy','audit'])[1 + abs(hashtext('a' || g)::bigint) % 8] \
                  || '.' || \
                (ARRAY['create','update','delete','run','share'])[1 + abs(hashtext('b' || g)::bigint) % 5], \
                (ARRAY['dashboard','query','pipeline','connector','alert','table'])[1 + abs(hashtext('k' || g)::bigint) % 6], \
                'res-' || (abs(hashtext('r' || g)::bigint) % 2000), \
                CASE WHEN abs(hashtext('o' || g)::bigint) % 100 < 70 THEN 'executed' \
                     WHEN abs(hashtext('o' || g)::bigint) % 100 < 85 THEN 'allowed' \
                     WHEN abs(hashtext('o' || g)::bigint) % 100 < 93 THEN 'refused' \
                     WHEN abs(hashtext('o' || g)::bigint) % 100 < 97 THEN 'failed' \
                     WHEN abs(hashtext('o' || g)::bigint) % 100 < 98 THEN 'approved' \
                     WHEN abs(hashtext('o' || g)::bigint) % 100 < 99 THEN 'rejected' \
                     WHEN abs(hashtext('o' || g)::bigint) % 200 < 199 THEN 'needs_confirmation' \
                     ELSE 'needs_approval' END \
         FROM generate_series(1, 50000) g",
    )
    .execute(pool)
    .await
    .unwrap();
    // The long tail: a person with 25 events in the whole year and an action
    // that happened 12 times. A filter on a rare value finds fewer rows than
    // one page holds, so a plan that walks `audit_event_at_idx` and discards
    // non-matches reads the whole index before it can stop.
    sqlx::query(
        "INSERT INTO audit_event (id, at, principal_id, principal_kind, action, outcome) \
         SELECT 'rare-p-' || g, timestamptz '2026-01-01 00:00:00+00' + g * interval '14 days', \
                'user-rare', 'user', 'dashboard.update', 'executed' \
         FROM generate_series(1, 25) g",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO audit_event (id, at, principal_id, principal_kind, action, outcome) \
         SELECT 'rare-a-' || g, timestamptz '2026-01-01 00:00:00+00' + g * interval '29 days', \
                'user-3', 'user', 'billing.invoice', 'executed' \
         FROM generate_series(1, 12) g",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("ANALYZE audit_event")
        .execute(pool)
        .await
        .unwrap();
}

#[sqlx::test(migrations = "../../migrations")]
async fn every_search_shape_is_served_by_an_index_on_a_large_table(
    pool: PgPool,
) -> sqlx::Result<()> {
    seed_realistic_table(&pool).await;

    let first_page = search(
        &pool,
        &AuditFilter {
            limit: 100,
            ..AuditFilter::default()
        },
        &AuditScope::All,
    )
    .await
    .unwrap();
    let cursor = first_page.next_cursor;

    // `Some(index)`: the plan must use that index. `None`: either the new
    // index or `audit_event_at_idx` serves the shape and the planner's pick
    // between them depends on the statistics sample, so only the absence of a
    // sequential scan is asserted.
    let shapes: Vec<(&str, AuditFilter, Option<&str>)> = vec![
        (
            "newest page, no filter",
            AuditFilter::default(),
            Some("audit_event_at_idx"),
        ),
        (
            "second page, no filter",
            AuditFilter {
                cursor,
                ..AuditFilter::default()
            },
            Some("audit_event_at_idx"),
        ),
        (
            "by person",
            AuditFilter {
                principal_id: Some("user-7".to_owned()),
                ..AuditFilter::default()
            },
            None,
        ),
        (
            "by a person with few events",
            AuditFilter {
                principal_id: Some("user-rare".to_owned()),
                ..AuditFilter::default()
            },
            Some("audit_event_principal_idx"),
        ),
        (
            "by a rare exact action",
            AuditFilter {
                action: Some("billing.invoice".to_owned()),
                ..AuditFilter::default()
            },
            Some("audit_event_action_idx"),
        ),
        (
            "by a rare action prefix",
            AuditFilter {
                action: Some("billing.".to_owned()),
                ..AuditFilter::default()
            },
            Some("audit_event_action_idx"),
        ),
        (
            "by exact action",
            AuditFilter {
                action: Some("dashboard.update".to_owned()),
                ..AuditFilter::default()
            },
            None,
        ),
        (
            "by action prefix",
            AuditFilter {
                action: Some("dashboard.".to_owned()),
                ..AuditFilter::default()
            },
            None,
        ),
        (
            "by outcome, 1 event in 12",
            AuditFilter {
                outcome: Some("refused".to_owned()),
                ..AuditFilter::default()
            },
            None,
        ),
        (
            "by outcome, 1 event in 100",
            AuditFilter {
                outcome: Some("rejected".to_owned()),
                ..AuditFilter::default()
            },
            None,
        ),
        (
            "by outcome, 1 event in 200",
            AuditFilter {
                outcome: Some("needs_approval".to_owned()),
                ..AuditFilter::default()
            },
            Some("audit_event_outcome_idx"),
        ),
        (
            "by one day",
            AuditFilter {
                since: Some(ts("2026-06-10T00:00:00Z")),
                until: Some(ts("2026-06-11T00:00:00Z")),
                ..AuditFilter::default()
            },
            Some("audit_event_at_idx"),
        ),
        (
            "by object",
            AuditFilter {
                resource_kind: Some("dashboard".to_owned()),
                resource_id: Some("res-17".to_owned()),
                ..AuditFilter::default()
            },
            Some("audit_event_resource_idx"),
        ),
        (
            "by person and one month",
            AuditFilter {
                principal_id: Some("user-7".to_owned()),
                since: Some(ts("2026-06-01T00:00:00Z")),
                until: Some(ts("2026-07-01T00:00:00Z")),
                ..AuditFilter::default()
            },
            None,
        ),
    ];

    let mut wrong = Vec::new();
    for (label, filter, index) in &shapes {
        let plan = explain_search(&pool, filter, false, &AuditScope::All)
            .await
            .unwrap()
            .join("\n");
        let ran = explain_search(&pool, filter, true, &AuditScope::All)
            .await
            .unwrap()
            .join("\n");
        println!("--- {label} (want {index:?})\n{plan}\n--- executed\n{ran}\n");
        let served = index.is_none_or(|name| plan.contains(name));
        if plan.contains("Seq Scan on audit_event") || !served {
            wrong.push(*label);
        }
    }
    assert!(
        wrong.is_empty(),
        "a sequential scan, or not the expected index, for: {wrong:?}"
    );
    Ok(())
}

// ── The reach over the trail (`SEC-20`, plan 100) ───────────────────────

/// The two seeded tenants the reach tests use: Meridian Group and
/// Meridian Retail.
const GROUP_TENANT: &str = "11111111-1111-4111-8111-000000000001";
const RETAIL_TENANT: &str = "11111111-1111-4111-8111-000000000002";

/// A person of `tenant`, as `app_user` and `app_user_tenant` hold one.
async fn a_person_of(pool: &PgPool, name: &str, email: &str, tenant: &str) -> String {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO app_user (username, name, email) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(email)
    .bind(name)
    .bind(email)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2)")
        .bind(id)
        .bind(tenant.parse::<Uuid>().unwrap())
        .execute(pool)
        .await
        .unwrap();
    id.to_string()
}

/// A row of the trail whose actor is `principal_id`.
async fn a_row_of(pool: &PgPool, action: &str, principal_id: Option<&str>) -> String {
    let mut event = sample(action);
    event.principal_id = principal_id.map(str::to_owned);
    // A row nobody signed in caused carries no actor at all.
    event.principal_kind = principal_id.map(|_| "user".to_owned());
    event.actor_label = principal_id.map(|_| "a person".to_owned());
    insert(pool, event).await.unwrap().id
}

/// The reach narrows the trail to the rows of the caller's own
/// organisation: another organisation's rows are not in the answer, and the
/// deployment's own rows (the ones nobody signed in caused) are in the
/// answer of the tenant that owns the shared catalog only.
#[sqlx::test(migrations = "../../migrations")]
async fn the_reach_narrows_the_trail_to_one_tenant(pool: PgPool) -> sqlx::Result<()> {
    let group = a_person_of(&pool, "Ayu", "ayu@example.test", GROUP_TENANT).await;
    let retail = a_person_of(&pool, "Rafi", "rafi@example.test", RETAIL_TENANT).await;
    let of_group = a_row_of(&pool, "row.of.group", Some(&group)).await;
    let of_retail = a_row_of(&pool, "row.of.retail", Some(&retail)).await;
    // Nobody signed in for this one: it is the deployment's own row.
    let of_the_deployment = a_row_of(&pool, "row.of.the.deployment", None).await;

    let scope_of_group = AuditScope::Tenant {
        tenant_id: GROUP_TENANT.to_owned(),
        owns_catalog: false,
    };
    let scope_of_retail = AuditScope::Tenant {
        tenant_id: RETAIL_TENANT.to_owned(),
        owns_catalog: false,
    };
    let scope_of_the_owner = AuditScope::Tenant {
        tenant_id: GROUP_TENANT.to_owned(),
        owns_catalog: true,
    };

    // A tenant that does not own the shared catalog reads its own rows and
    // nobody else's — not the other organisation's, not the deployment's.
    let retail_rows = list(&pool, AuditFilter::default(), &scope_of_retail)
        .await
        .unwrap();
    assert_eq!(
        retail_rows
            .iter()
            .map(|e| e.id.as_str())
            .collect::<Vec<_>>(),
        [of_retail.as_str()],
        "a tenant reads its own rows only"
    );

    // The catalog owner reads its own rows and the deployment's own rows.
    let owner_rows = list(&pool, AuditFilter::default(), &scope_of_the_owner)
        .await
        .unwrap();
    let owner_ids: Vec<&str> = owner_rows.iter().map(|e| e.id.as_str()).collect();
    assert!(
        owner_ids.contains(&of_group.as_str()),
        "the owner reads its own rows: {owner_ids:?}"
    );
    assert!(
        owner_ids.contains(&of_the_deployment.as_str()),
        "the owner reads the deployment's own rows: {owner_ids:?}"
    );
    assert!(
        !owner_ids.contains(&of_retail.as_str()),
        "the owner reads no other organisation's row: {owner_ids:?}"
    );

    // A caller that belongs to no tenant reads no row at all.
    let nobody = list(&pool, AuditFilter::default(), &AuditScope::NoTenant)
        .await
        .unwrap();
    assert!(nobody.is_empty(), "no tenant, no row: {nobody:?}");

    // An unrestricted caller reads every row.
    let everyone = list(&pool, AuditFilter::default(), &AuditScope::All)
        .await
        .unwrap();
    assert_eq!(everyone.len(), 3, "every row of the trail");

    // The count and the single read agree with the list.
    let counted = count_matching(&pool, &AuditFilter::default(), &scope_of_retail)
        .await
        .unwrap();
    assert_eq!(counted, 1, "the count agrees with the list");
    let one = get(&pool, &of_retail, &scope_of_group).await.unwrap();
    assert!(
        one.is_none(),
        "a row out of the reach is the same as no row at all"
    );
    let one = get(&pool, &of_retail, &scope_of_retail).await.unwrap();
    assert_eq!(one.map(|e| e.id), Some(of_retail.clone()));
    Ok(())
}

// ── The actors of the trail (`GOV-1` acceptance follow-up, D2) ──────────

/// The person filter's list is capped: 501 actors seeded, exactly
/// [`ACTORS_MAX`] come back, newest-seen first, and the one left out is the
/// actor seen longest ago.
#[sqlx::test(migrations = "../../migrations")]
async fn actors_lists_at_most_five_hundred_actors_newest_seen_first(
    pool: PgPool,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO audit_event (id, at, principal_id, principal_kind, actor_label, action, \
         outcome) \
         SELECT 'actor-cap-' || lpad(g::text, 4, '0'), \
                timestamptz '2026-01-01T00:00:00Z' + g * interval '1 second', \
                'principal-' || lpad(g::text, 4, '0'), 'user', 'Actor ' || g, 'a.b', 'executed' \
         FROM generate_series(1, 501) AS g",
    )
    .execute(&pool)
    .await
    .unwrap();

    let listed = actors(&pool, &AuditScope::All).await.unwrap();
    assert_eq!(ACTORS_MAX, 500, "the cap this test pins");
    assert_eq!(listed.len(), 500, "the list is capped at {ACTORS_MAX}");
    assert_eq!(
        listed[0].principal_id, "principal-0501",
        "newest seen first"
    );
    assert_eq!(
        listed[1].principal_id, "principal-0500",
        "second newest next"
    );
    assert_eq!(
        listed.last().map(|a| a.principal_id.as_str()),
        Some("principal-0002"),
        "the actor seen longest ago is the one left out"
    );
    Ok(())
}

/// A row nobody signed in caused carries no `principal_id`, and the filter
/// sends an id back, so an id-less actor cannot be picked: it is not listed,
/// however often it acted.
#[sqlx::test(migrations = "../../migrations")]
async fn actors_leaves_out_the_rows_nobody_signed_in_caused(pool: PgPool) -> sqlx::Result<()> {
    let mut named = sample("a.b");
    named.principal_id = Some("principal-1".to_owned());
    insert(&pool, named).await.unwrap();
    let mut anonymous = sample("a.b");
    anonymous.principal_id = None;
    anonymous.principal_kind = None;
    anonymous.actor_label = None;
    insert(&pool, anonymous).await.unwrap();

    let listed = actors(&pool, &AuditScope::All).await.unwrap();
    assert_eq!(
        listed
            .iter()
            .map(|a| a.principal_id.as_str())
            .collect::<Vec<_>>(),
        ["principal-1"],
        "only the actor with an id is listed: {listed:?}"
    );
    Ok(())
}

/// The reach narrows the actor list the same way it narrows the trail: a
/// tenant that does not own the shared catalog lists only its own people,
/// and no scope lists the deployment's own rows — they carry no principal id
/// at all.
#[sqlx::test(migrations = "../../migrations")]
async fn the_reach_narrows_the_actor_list_to_one_tenant(pool: PgPool) -> sqlx::Result<()> {
    let group = a_person_of(&pool, "Ayu", "ayu@example.test", GROUP_TENANT).await;
    let retail = a_person_of(&pool, "Rafi", "rafi@example.test", RETAIL_TENANT).await;
    a_row_of(&pool, "row.of.group", Some(&group)).await;
    a_row_of(&pool, "row.of.retail", Some(&retail)).await;
    a_row_of(&pool, "row.of.the.deployment", None).await;

    let scope_of_retail = AuditScope::Tenant {
        tenant_id: RETAIL_TENANT.to_owned(),
        owns_catalog: false,
    };
    let listed = actors(&pool, &scope_of_retail).await.unwrap();
    assert_eq!(
        listed
            .iter()
            .map(|a| a.principal_id.as_str())
            .collect::<Vec<_>>(),
        [retail.as_str()],
        "a tenant lists its own people only"
    );

    let everyone = actors(&pool, &AuditScope::All).await.unwrap();
    let ids: Vec<&str> = everyone.iter().map(|a| a.principal_id.as_str()).collect();
    assert!(
        ids.contains(&group.as_str()) && ids.contains(&retail.as_str()),
        "{ids:?}"
    );
    assert_eq!(
        ids.len(),
        2,
        "the deployment's own rows are no actor: {ids:?}"
    );
    Ok(())
}
