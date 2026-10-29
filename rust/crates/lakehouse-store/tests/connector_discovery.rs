//! Integration tests for `lakehouse_store::connector_discovery` against a real
//! Postgres (`SRC-13` task 1; same setup as `tests/connector_run.rs`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_test_support as _;

use lakehouse_store::StoreError;
use lakehouse_store::connector_discovery::{
    Answer, ErrorClass, Kind, ListedColumn, ListedObject, Listing, MAX_COLUMNS, MAX_OBJECTS,
    ObjectKind, Outcome, Preview, STALE_AFTER_SECONDS, State, Tested, begin, finish, get,
};
use sqlx::PgPool;
use time::{Duration, OffsetDateTime};

const PG: &str = "conn-pg-lakehouse";

fn listing(names: &[&str]) -> Listing {
    Listing {
        objects: names
            .iter()
            .map(|name| ListedObject::named(*name, ObjectKind::Table))
            .collect(),
        truncated: false,
    }
}

fn preview() -> Preview {
    Preview {
        columns: vec![
            ListedColumn {
                name: "id".to_owned(),
                type_name: "integer".to_owned(),
                inferred: false,
            },
            ListedColumn {
                name: "note".to_owned(),
                type_name: "string".to_owned(),
                inferred: true,
            },
        ],
        primary_key: Some(vec!["id".to_owned()]),
    }
}

/// `SRC-13` D1: a request is `running` until its result is stored, and the
/// stored result is exactly what was sent.
#[sqlx::test(migrations = "../../migrations")]
async fn a_request_runs_then_holds_its_result(pool: PgPool) -> sqlx::Result<()> {
    let t0 = OffsetDateTime::now_utc();
    begin(&pool, PG, Kind::List, "req-1", "", "public", t0)
        .await
        .unwrap();
    let running = get(&pool, PG, Kind::List).await.unwrap().unwrap();
    assert_eq!(running.state, State::Running);
    assert!(running.listing.is_none());

    let sent = listing(&["public.a", "public.b"]);
    assert!(
        finish(
            &pool,
            PG,
            Kind::List,
            "req-1",
            &Outcome::Listing(sent.clone()),
            t0 + Duration::seconds(3)
        )
        .await
        .unwrap()
    );
    let done = get(&pool, PG, Kind::List).await.unwrap().unwrap();
    assert_eq!(done.state, State::Done);
    assert_eq!(done.listing, Some(sent));
    assert_eq!(done.scope, "public");
    assert!(done.ended_at.is_some());
    Ok(())
}

/// `SRC-13` D1: the first post wins; a repeat, and a post that carries another
/// request's id, store nothing.
#[sqlx::test(migrations = "../../migrations")]
async fn a_repeated_or_foreign_result_is_not_stored(pool: PgPool) -> sqlx::Result<()> {
    let t0 = OffsetDateTime::now_utc();
    begin(&pool, PG, Kind::List, "req-1", "", "", t0)
        .await
        .unwrap();
    let wrong = finish(
        &pool,
        PG,
        Kind::List,
        "req-other",
        &Outcome::Listing(listing(&["x.y"])),
        t0,
    )
    .await
    .unwrap();
    assert!(!wrong, "another request's id stores nothing");
    assert!(
        finish(
            &pool,
            PG,
            Kind::List,
            "req-1",
            &Outcome::Listing(listing(&["first.one"])),
            t0
        )
        .await
        .unwrap()
    );
    let again = finish(
        &pool,
        PG,
        Kind::List,
        "req-1",
        &Outcome::Listing(listing(&["second.one"])),
        t0,
    )
    .await
    .unwrap();
    assert!(!again, "the first post wins");
    let stored = get(&pool, PG, Kind::List).await.unwrap().unwrap();
    assert_eq!(stored.listing.unwrap().objects[0].name, "first.one");
    Ok(())
}

/// `SRC-13` D1: the latest request wins. A new request replaces the row, and
/// the old request's late post is dropped.
#[sqlx::test(migrations = "../../migrations")]
async fn a_new_request_replaces_the_row_and_a_late_post_is_dropped(
    pool: PgPool,
) -> sqlx::Result<()> {
    let t0 = OffsetDateTime::now_utc();
    begin(&pool, PG, Kind::Preview, "req-1", "public.a", "", t0)
        .await
        .unwrap();
    begin(&pool, PG, Kind::Preview, "req-2", "public.b", "", t0)
        .await
        .unwrap();
    let late = finish(
        &pool,
        PG,
        Kind::Preview,
        "req-1",
        &Outcome::Preview(preview()),
        t0,
    )
    .await
    .unwrap();
    assert!(!late);
    let row = get(&pool, PG, Kind::Preview).await.unwrap().unwrap();
    assert_eq!(row.object, "public.b");
    assert_eq!(row.state, State::Running);

    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM connector_discovery")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 1, "one row per connector and kind");
    Ok(())
}

/// `SRC-13` D10: a failure is a class from the closed list, and no result.
#[sqlx::test(migrations = "../../migrations")]
async fn a_failure_stores_only_its_class(pool: PgPool) -> sqlx::Result<()> {
    let t0 = OffsetDateTime::now_utc();
    begin(&pool, PG, Kind::List, "req-1", "", "", t0)
        .await
        .unwrap();
    assert!(
        finish(
            &pool,
            PG,
            Kind::List,
            "req-1",
            &Outcome::Failed(ErrorClass::NoPermission),
            t0
        )
        .await
        .unwrap()
    );
    let stored = get(&pool, PG, Kind::List).await.unwrap().unwrap();
    assert_eq!(stored.state, State::Failed);
    assert_eq!(stored.error_class, Some(ErrorClass::NoPermission));
    assert!(stored.listing.is_none());

    let answer = stored.into_answer(t0);
    assert_eq!(answer.reason, Some(ErrorClass::NoPermission.reason()));
    let json = serde_json::to_value(&answer).unwrap();
    assert_eq!(json["errorClass"], "no_permission");
    assert_eq!(json["state"], "failed");
    assert!(json.get("objects").is_none());
    Ok(())
}

/// `SRC-13` D10, principle 4: the table itself refuses a class outside the
/// closed list and a state that does not fit its contents, whatever the code
/// above it does.
#[sqlx::test(migrations = "../../migrations")]
async fn the_table_refuses_a_free_text_class_and_a_state_that_lies(
    pool: PgPool,
) -> sqlx::Result<()> {
    let t0 = OffsetDateTime::now_utc();
    let free_text = sqlx::query(
        "INSERT INTO connector_discovery \
           (connector_id, kind, request_id, state, error_class, started_at, ended_at) \
         VALUES ($1, 'list', 'r', 'failed', 'password authentication failed for user x', $2, $2)",
    )
    .bind(PG)
    .bind(t0)
    .execute(&pool)
    .await;
    assert!(free_text.is_err(), "a free-text class is refused");

    let done_without_result = sqlx::query(
        "INSERT INTO connector_discovery \
           (connector_id, kind, request_id, state, started_at, ended_at) \
         VALUES ($1, 'list', 'r', 'done', $2, $2)",
    )
    .bind(PG)
    .bind(t0)
    .execute(&pool)
    .await;
    assert!(done_without_result.is_err(), "done needs a result");

    let object_on_a_list = sqlx::query(
        "INSERT INTO connector_discovery (connector_id, kind, request_id, object, started_at) \
         VALUES ($1, 'list', 'r', 'public.a', $2)",
    )
    .bind(PG)
    .bind(t0)
    .execute(&pool)
    .await;
    assert!(object_on_a_list.is_err(), "a list names no object");
    Ok(())
}

/// `SRC-13` D9: the bounds are enforced before anything is stored.
#[sqlx::test(migrations = "../../migrations")]
async fn a_result_past_a_bound_or_with_a_bad_string_is_refused(pool: PgPool) -> sqlx::Result<()> {
    let t0 = OffsetDateTime::now_utc();
    begin(&pool, PG, Kind::List, "req-1", "", "", t0)
        .await
        .unwrap();

    let too_many = Listing {
        objects: (0..=MAX_OBJECTS)
            .map(|i| ListedObject::named(format!("t{i}"), ObjectKind::Table))
            .collect(),
        truncated: false,
    };
    let long_name = "n".repeat(257);
    let bad_sets = [
        too_many,
        listing(&["   "]),
        listing(&["line\nbreak"]),
        listing(&[long_name.as_str()]),
        Listing {
            objects: vec![ListedObject {
                modified_at: Some("yesterday".to_owned()),
                ..ListedObject::named("f.csv", ObjectKind::File)
            }],
            truncated: false,
        },
        Listing {
            objects: vec![ListedObject {
                size: Some(-1),
                ..ListedObject::named("f.csv", ObjectKind::File)
            }],
            truncated: false,
        },
        // SRC-3: a load description is short text without control characters.
        Listing {
            objects: vec![ListedObject {
                load: Some("re-read in full: a\nb".to_owned()),
                ..ListedObject::named("tickets", ObjectKind::Object)
            }],
            truncated: false,
        },
        Listing {
            objects: vec![ListedObject {
                load: Some("x".repeat(201)),
                ..ListedObject::named("tickets", ObjectKind::Object)
            }],
            truncated: false,
        },
        // SRC-3 slice 5 review BLOCKER 14: a label is bounded like a name.
        Listing {
            objects: vec![ListedObject {
                label: Some("Team\nSite / Tasks".to_owned()),
                ..ListedObject::named("site/list", ObjectKind::Object)
            }],
            truncated: false,
        },
        Listing {
            objects: vec![ListedObject {
                label: Some("x".repeat(257)),
                ..ListedObject::named("site/list", ObjectKind::Object)
            }],
            truncated: false,
        },
        Listing {
            objects: vec![ListedObject {
                label: Some("  ".to_owned()),
                ..ListedObject::named("site/list", ObjectKind::Object)
            }],
            truncated: false,
        },
    ];
    for bad in bad_sets {
        let err = finish(&pool, PG, Kind::List, "req-1", &Outcome::Listing(bad), t0)
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::Validation(_)), "{err:?}");
    }
    // Nothing was stored by any refusal: the request is still running.
    let row = get(&pool, PG, Kind::List).await.unwrap().unwrap();
    assert_eq!(row.state, State::Running);

    let too_many_columns = Preview {
        columns: (0..=MAX_COLUMNS)
            .map(|i| ListedColumn {
                name: format!("c{i}"),
                type_name: "text".to_owned(),
                inferred: false,
            })
            .collect(),
        primary_key: None,
    };
    begin(&pool, PG, Kind::Preview, "req-p", "public.a", "", t0)
        .await
        .unwrap();
    let err = finish(
        &pool,
        PG,
        Kind::Preview,
        "req-p",
        &Outcome::Preview(too_many_columns),
        t0,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, StoreError::Validation(_)), "{err:?}");

    // A source with nothing to sample is no columns, not a refusal (D6).
    let nothing_to_sample = Preview {
        columns: vec![],
        primary_key: None,
    };
    assert!(
        finish(
            &pool,
            PG,
            Kind::Preview,
            "req-p",
            &Outcome::Preview(nothing_to_sample),
            t0
        )
        .await
        .unwrap()
    );
    let stored = get(&pool, PG, Kind::Preview).await.unwrap().unwrap();
    assert_eq!(stored.state, State::Done);
    assert_eq!(
        stored.preview.unwrap().columns,
        [] as [lakehouse_store::connector_discovery::ListedColumn; 0]
    );
    Ok(())
}

/// A list result cannot end a preview request, nor the reverse.
#[sqlx::test(migrations = "../../migrations")]
async fn a_result_of_the_wrong_kind_is_refused(pool: PgPool) -> sqlx::Result<()> {
    let t0 = OffsetDateTime::now_utc();
    begin(&pool, PG, Kind::Preview, "req-1", "public.a", "", t0)
        .await
        .unwrap();
    let err = finish(
        &pool,
        PG,
        Kind::Preview,
        "req-1",
        &Outcome::Listing(listing(&["x"])),
        t0,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, StoreError::Validation(_)));
    let err = begin(&pool, PG, Kind::List, "req-2", "public.a", "", t0)
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::Validation(_)),
        "a list names no object"
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_request_for_an_unknown_connector_is_a_foreign_key_violation(
    pool: PgPool,
) -> sqlx::Result<()> {
    let err = begin(
        &pool,
        "no-such-connector",
        Kind::List,
        "req-1",
        "",
        "",
        OffsetDateTime::now_utc(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, StoreError::ForeignKeyViolation), "{err:?}");
    Ok(())
}

/// `SRC-13` D1: a request whose job never posted is shown as a timeout after
/// the stale window, and not before.
#[sqlx::test(migrations = "../../migrations")]
async fn a_request_that_never_finished_is_shown_as_a_timeout_after_the_window(
    pool: PgPool,
) -> sqlx::Result<()> {
    let t0 = OffsetDateTime::now_utc();
    begin(&pool, PG, Kind::List, "req-1", "", "", t0)
        .await
        .unwrap();
    let stored = get(&pool, PG, Kind::List).await.unwrap().unwrap();

    let fresh = stored
        .clone()
        .into_answer(t0 + Duration::seconds(STALE_AFTER_SECONDS));
    assert_eq!(fresh.state, State::Running);

    let stale = stored.into_answer(t0 + Duration::seconds(STALE_AFTER_SECONDS + 1));
    assert_eq!(stale.state, State::Failed);
    assert_eq!(stale.error_class, Some(ErrorClass::Timeout));
    assert_eq!(stale.reason, Some(ErrorClass::Timeout.reason()));
    Ok(())
}

/// The wire shape of a finished preview and of a cut list (module docs).
#[test]
fn answers_serialize_to_the_documented_shapes() {
    let answer = Answer::preview("public.orders", preview());
    let json = serde_json::to_value(&answer).unwrap();
    assert_eq!(json["kind"], "preview");
    assert_eq!(json["state"], "done");
    assert_eq!(json["object"], "public.orders");
    assert_eq!(json["columns"][0]["typeName"], "integer");
    assert_eq!(json["columns"][1]["inferred"], true);
    assert_eq!(json["primaryKey"][0], "id");
    assert!(json.get("objects").is_none());

    let cut = Answer::listing(
        "sales/*",
        Listing {
            objects: vec![ListedObject {
                size: Some(10),
                modified_at: Some("2026-10-15T09:00:00Z".to_owned()),
                ..ListedObject::named("sales/a.csv", ObjectKind::File)
            }],
            truncated: true,
        },
    );
    let json = serde_json::to_value(&cut).unwrap();
    assert_eq!(json["scope"], "sales/*");
    assert_eq!(json["truncated"], true);
    assert_eq!(json["objects"][0]["kind"], "file");
    assert_eq!(json["objects"][0]["modifiedAt"], "2026-10-15T09:00:00Z");
    assert!(json["reason"].as_str().unwrap().contains("10,000"));

    // BLOCKER 14: a label and a site are carried; an object without a label says nothing of one.
    let labelled = Answer::listing(
        "",
        Listing {
            objects: vec![
                ListedObject {
                    label: Some("Team / Tasks".to_owned()),
                    ..ListedObject::named("site,1,2/list", ObjectKind::Object)
                },
                ListedObject::named("site,1,2", ObjectKind::Site),
            ],
            truncated: false,
        },
    );
    let json = serde_json::to_value(&labelled).unwrap();
    assert_eq!(json["objects"][0]["label"], "Team / Tasks");
    assert_eq!(json["objects"][1]["kind"], "site");
    assert!(json["objects"][1].get("label").is_none());

    let running = serde_json::to_value(Answer::running(Kind::List)).unwrap();
    assert_eq!(
        running,
        serde_json::json!({"kind": "list", "state": "running"})
    );
}

/// `SRC-4` T1.1 (migration 0115): a connection test has a row of its own, so it
/// never overwrites the person's list; its result is the connection time, a
/// wrong shape is refused, and a test names no object.
#[sqlx::test(migrations = "../../migrations")]
async fn a_connection_test_has_its_own_row_and_holds_only_a_latency(
    pool: PgPool,
) -> sqlx::Result<()> {
    let t0 = OffsetDateTime::now_utc();
    begin(&pool, PG, Kind::List, "list-1", "", "public", t0)
        .await
        .unwrap();
    finish(
        &pool,
        PG,
        Kind::List,
        "list-1",
        &Outcome::Listing(listing(&["public.a"])),
        t0,
    )
    .await
    .unwrap();

    begin(&pool, PG, Kind::Test, "test-1", "", "", t0)
        .await
        .unwrap();
    let running = get(&pool, PG, Kind::Test).await.unwrap().unwrap();
    assert_eq!(running.state, State::Running);

    // A listing is not a test result, and a latency past the bound is not a
    // measurement.
    let wrong = finish(
        &pool,
        PG,
        Kind::Test,
        "test-1",
        &Outcome::Listing(listing(&["x"])),
        t0,
    )
    .await;
    assert!(matches!(wrong, Err(StoreError::Validation(_))));
    let absurd = finish(
        &pool,
        PG,
        Kind::Test,
        "test-1",
        &Outcome::Tested(Tested {
            latency_ms: 3_600_001,
        }),
        t0,
    )
    .await;
    assert!(matches!(absurd, Err(StoreError::Validation(_))));

    assert!(
        finish(
            &pool,
            PG,
            Kind::Test,
            "test-1",
            &Outcome::Tested(Tested { latency_ms: 212 }),
            t0,
        )
        .await
        .unwrap()
    );
    let answer = get(&pool, PG, Kind::Test)
        .await
        .unwrap()
        .unwrap()
        .into_answer(t0);
    assert_eq!(answer.kind, Kind::Test);
    assert_eq!(answer.state, State::Done);
    assert_eq!(answer.latency_ms, Some(212));
    assert!(answer.objects.is_none());

    // The list is untouched.
    let list = get(&pool, PG, Kind::List).await.unwrap().unwrap();
    assert_eq!(list.state, State::Done);
    assert!(list.listing.is_some());

    // A failed test holds a class; a test names no object.
    begin(&pool, PG, Kind::Test, "test-2", "", "", t0)
        .await
        .unwrap();
    finish(
        &pool,
        PG,
        Kind::Test,
        "test-2",
        &Outcome::Failed(ErrorClass::SignInRefused),
        t0,
    )
    .await
    .unwrap();
    let failed = get(&pool, PG, Kind::Test)
        .await
        .unwrap()
        .unwrap()
        .into_answer(t0);
    assert_eq!(failed.error_class, Some(ErrorClass::SignInRefused));
    assert!(
        begin(&pool, PG, Kind::Test, "test-3", "public.a", "", t0)
            .await
            .is_err()
    );
    Ok(())
}
