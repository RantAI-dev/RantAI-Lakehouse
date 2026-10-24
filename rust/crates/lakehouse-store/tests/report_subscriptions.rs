//! Integration tests for `lakehouse_store::report_subscriptions` against a real
//! Postgres (`#[sqlx::test]`, see `tests/reporting_settings.rs` for how the
//! database is provided). `RPT-1` T1.

#![allow(clippy::unwrap_used, clippy::expect_used)]

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap runs for this test binary.
use lakehouse_test_support as _;

use lakehouse_store::StoreError;
use lakehouse_store::report_schedule::Schedule;
use lakehouse_store::report_subscriptions::{
    DeliveryStatus, NewDelivery, NewSubscription, PAUSE_AFTER_FAILURES, Recipient,
    UNSCHEDULABLE_REASON, claim_due, create, delete, get, last_deliveries, list_deliveries,
    list_for_board, list_for_owner, note_outcome, prune_deliveries, recipients, record_delivery,
    retry_at, set_paused, update,
};
use serde_json::json;
use sqlx::PgPool;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

async fn user(pool: &PgPool, name: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO app_user (name, email) VALUES ($1, $2) RETURNING id")
        .bind(name)
        .bind(format!("{name}@example.com"))
        .fetch_one(pool)
        .await
        .unwrap()
}

fn new_sub(owner: Uuid, board: &str, next_run_at: OffsetDateTime) -> NewSubscription {
    NewSubscription {
        board: board.to_owned(),
        owner_id: owner,
        schedule: Schedule::Weekly {
            weekday: 1,
            hour: 8,
            minute: 0,
        },
        time_zone: "Asia/Jakarta".to_owned(),
        attachments: json!({ "pdf": { "paper": "a4", "orientation": "portrait" }, "charts": [] }),
        filters: Some(json!([{ "column": "region", "op": "eq", "value": "Bali" }])),
        params: None,
        grain: None,
        skip_when_empty: false,
        next_run_at,
        recipients: vec![Recipient::Email("Outside@Example.COM".to_owned())],
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_subscription_round_trips_with_its_schedule_values_and_recipients(
    pool: PgPool,
) -> sqlx::Result<()> {
    let owner = user(&pool, "owner").await;
    let colleague = user(&pool, "colleague").await;
    let at = OffsetDateTime::now_utc() + Duration::hours(1);
    let mut new = new_sub(owner, "main", at);
    new.recipients.push(Recipient::User(colleague));
    let made = create(&pool, &new).await.expect("create");

    assert_eq!(made.board, "main");
    assert_eq!(made.owner_id, owner);
    assert_eq!(made.schedule(), Ok(new.schedule));
    assert_eq!(made.filters, new.filters);
    assert!(!made.paused && made.consecutive_failures == 0);
    assert_eq!(get(&pool, made.id).await.unwrap(), Some(made.clone()));
    assert_eq!(list_for_board(&pool, "main").await.unwrap().len(), 1);
    assert!(list_for_board(&pool, "other").await.unwrap().is_empty());
    assert_eq!(list_for_owner(&pool, owner).await.unwrap().len(), 1);

    let got = recipients(&pool, made.id).await.unwrap();
    assert_eq!(got.len(), 2);
    let outside = got.iter().find(|r| r.email.is_some()).unwrap();
    assert_eq!(
        outside.email.as_deref(),
        Some("outside@example.com"),
        "an address is stored lower-cased"
    );
    let inside = got.iter().find(|r| r.user_id.is_some()).unwrap();
    assert_eq!(inside.user_name.as_deref(), Some("colleague"));
    assert_eq!(inside.user_status.as_deref(), Some("active"));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_recipient_is_a_user_or_an_address_never_both_and_never_twice(
    pool: PgPool,
) -> sqlx::Result<()> {
    let owner = user(&pool, "owner").await;
    let made = create(&pool, &new_sub(owner, "main", OffsetDateTime::now_utc()))
        .await
        .unwrap();

    // The table itself refuses both-set and neither-set rows.
    for (user_id, email) in [
        (Some(owner), Some("a@example.com")),
        (None::<Uuid>, None::<&str>),
    ] {
        let refused = sqlx::query(
            "INSERT INTO report_recipient (subscription_id, user_id, email) VALUES ($1, $2, $3)",
        )
        .bind(made.id)
        .bind(user_id)
        .bind(email)
        .execute(&pool)
        .await;
        assert!(refused.is_err(), "{user_id:?} {email:?}");
    }
    // The same address twice (any case) and the same user twice are conflicts.
    let mut dup = new_sub(owner, "main", OffsetDateTime::now_utc());
    dup.recipients = vec![
        Recipient::Email("x@example.com".to_owned()),
        Recipient::Email("X@EXAMPLE.com".to_owned()),
    ];
    assert!(matches!(
        create(&pool, &dup).await,
        Err(StoreError::Conflict)
    ));
    dup.recipients = vec![Recipient::User(owner), Recipient::User(owner)];
    assert!(matches!(
        create(&pool, &dup).await,
        Err(StoreError::Conflict)
    ));
    // A user that does not exist is a foreign-key violation.
    dup.recipients = vec![Recipient::User(Uuid::new_v4())];
    assert!(matches!(
        create(&pool, &dup).await,
        Err(StoreError::ForeignKeyViolation)
    ));
    // The failed creates left nothing behind.
    assert_eq!(list_for_owner(&pool, owner).await.unwrap().len(), 1);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_row_must_carry_exactly_the_fields_its_kind_uses(pool: PgPool) -> sqlx::Result<()> {
    let owner = user(&pool, "owner").await;
    for (kind, hour, weekday, month_day) in [
        ("hourly", Some(8_i16), None::<i16>, None::<i16>),
        ("daily", None, None, None),
        ("weekly", Some(8), None, None),
        ("monthly", Some(8), Some(1), Some(1)),
        ("monthly", Some(8), None, Some(29)),
    ] {
        let refused = sqlx::query(
            "INSERT INTO report_subscription (board, owner_id, kind, minute, hour, weekday, \
             month_day, time_zone, attachments, next_run_at) \
             VALUES ('main', $1, $2, 0, $3, $4, $5, 'UTC', '{}', now())",
        )
        .bind(owner)
        .bind(kind)
        .bind(hour)
        .bind(weekday)
        .bind(month_day)
        .execute(&pool)
        .await;
        assert!(
            refused.is_err(),
            "{kind} {hour:?} {weekday:?} {month_day:?}"
        );
    }
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn update_replaces_the_schedule_and_recipients_and_keeps_owner_and_board(
    pool: PgPool,
) -> sqlx::Result<()> {
    let owner = user(&pool, "owner").await;
    let other = user(&pool, "other").await;
    let made = create(&pool, &new_sub(owner, "main", OffsetDateTime::now_utc()))
        .await
        .unwrap();
    let mut changed = new_sub(
        other,
        "elsewhere",
        OffsetDateTime::now_utc() + Duration::days(2),
    );
    changed.schedule = Schedule::Hourly { minute: 15 };
    changed.skip_when_empty = true;
    changed.recipients = vec![Recipient::User(other)];
    let got = update(&pool, made.id, &changed).await.unwrap().unwrap();
    assert_eq!(got.schedule(), Ok(Schedule::Hourly { minute: 15 }));
    assert!(got.skip_when_empty);
    assert_eq!(got.owner_id, owner, "the owner never changes");
    assert_eq!(got.board, "main", "the board never changes");
    let r = recipients(&pool, made.id).await.unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].user_id, Some(other));
    assert!(
        update(&pool, Uuid::new_v4(), &changed)
            .await
            .unwrap()
            .is_none()
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn claiming_takes_only_due_unpaused_rows_and_moves_them_forward(
    pool: PgPool,
) -> sqlx::Result<()> {
    let owner = user(&pool, "owner").await;
    let now = OffsetDateTime::now_utc();
    let due = create(&pool, &new_sub(owner, "a", now - Duration::minutes(3)))
        .await
        .unwrap();
    let _later = create(&pool, &new_sub(owner, "b", now + Duration::hours(1)))
        .await
        .unwrap();
    let paused = create(&pool, &new_sub(owner, "c", now - Duration::minutes(3)))
        .await
        .unwrap();
    set_paused(&pool, paused.id, true, Some("because"), None)
        .await
        .unwrap();

    let next = now + Duration::days(7);
    let claimed = claim_due(&pool, now, 5, |_| Some(next)).await.unwrap();
    assert_eq!(claimed.iter().map(|s| s.id).collect::<Vec<_>>(), [due.id]);
    assert!(
        claimed[0].next_run_at < now,
        "the row returned still holds the time it was due"
    );
    let stored = get(&pool, due.id).await.unwrap().unwrap();
    assert!(
        (stored.next_run_at - next).abs() < Duration::milliseconds(5),
        "the stored time moved forward"
    );
    // A second claim finds nothing: a missed run happens once.
    assert!(
        claim_due(&pool, now, 5, |_| Some(next))
            .await
            .unwrap()
            .is_empty()
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn claiming_respects_the_limit_oldest_first(pool: PgPool) -> sqlx::Result<()> {
    let owner = user(&pool, "owner").await;
    let now = OffsetDateTime::now_utc();
    let mut ids = Vec::new();
    for minutes in [30, 10, 20] {
        let s = create(
            &pool,
            &new_sub(
                owner,
                &format!("b{minutes}"),
                now - Duration::minutes(minutes),
            ),
        )
        .await
        .unwrap();
        ids.push((minutes, s.id));
    }
    let claimed = claim_due(&pool, now, 2, |_| Some(now + Duration::days(1)))
        .await
        .unwrap();
    let oldest_two: Vec<Uuid> = [30, 20]
        .iter()
        .map(|m| ids.iter().find(|(x, _)| x == m).unwrap().1)
        .collect();
    assert_eq!(claimed.iter().map(|s| s.id).collect::<Vec<_>>(), oldest_two);
    Ok(())
}

/// D3: a row another claim has locked is skipped, not waited for and not taken
/// twice.
#[sqlx::test(migrations = "../../migrations")]
async fn a_row_locked_by_another_claim_is_skipped(pool: PgPool) -> sqlx::Result<()> {
    let owner = user(&pool, "owner").await;
    let now = OffsetDateTime::now_utc();
    for i in 0..4 {
        create(
            &pool,
            &new_sub(owner, &format!("b{i}"), now - Duration::minutes(5)),
        )
        .await
        .unwrap();
    }
    // The first "replica" holds every due row locked.
    let mut holder = pool.begin().await?;
    let locked: Vec<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM report_subscription WHERE NOT paused AND next_run_at <= $1 \
         FOR UPDATE",
    )
    .bind(now)
    .fetch_all(&mut *holder)
    .await?;
    assert_eq!(locked.len(), 4);
    // The second claims on its own connection and gets nothing, without waiting.
    let started = std::time::Instant::now();
    let second = claim_due(&pool, now, 5, |_| Some(now + Duration::days(1)))
        .await
        .unwrap();
    assert!(second.is_empty(), "locked rows are skipped");
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    holder.rollback().await?;
    // Once released, they are claimable.
    let third = claim_due(&pool, now, 5, |_| Some(now + Duration::days(1)))
        .await
        .unwrap();
    assert_eq!(third.len(), 4);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn two_claims_at_once_get_disjoint_rows(pool: PgPool) -> sqlx::Result<()> {
    let owner = user(&pool, "owner").await;
    let now = OffsetDateTime::now_utc();
    for i in 0..12 {
        create(
            &pool,
            &new_sub(owner, &format!("b{i}"), now - Duration::minutes(5)),
        )
        .await
        .unwrap();
    }
    let later = now + Duration::days(1);
    let (a, b) = tokio::join!(
        claim_due(&pool, now, 5, |_| Some(later)),
        claim_due(&pool, now, 5, |_| Some(later)),
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    let mut all: Vec<Uuid> = a.iter().chain(b.iter()).map(|s| s.id).collect();
    let total = all.len();
    all.sort();
    all.dedup();
    assert_eq!(all.len(), total, "no row was claimed twice");
    assert!(total <= 10);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_row_whose_schedule_cannot_be_worked_out_is_paused_with_a_reason(
    pool: PgPool,
) -> sqlx::Result<()> {
    let owner = user(&pool, "owner").await;
    let now = OffsetDateTime::now_utc();
    let s = create(&pool, &new_sub(owner, "a", now - Duration::minutes(1)))
        .await
        .unwrap();
    assert!(claim_due(&pool, now, 5, |_| None).await.unwrap().is_empty());
    let stored = get(&pool, s.id).await.unwrap().unwrap();
    assert!(stored.paused);
    assert_eq!(stored.paused_reason.as_deref(), Some(UNSCHEDULABLE_REASON));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn five_failures_in_a_row_pause_it_once_and_a_success_resets_the_count(
    pool: PgPool,
) -> sqlx::Result<()> {
    let owner = user(&pool, "owner").await;
    let s = create(&pool, &new_sub(owner, "a", OffsetDateTime::now_utc()))
        .await
        .unwrap();
    for n in 1..PAUSE_AFTER_FAILURES {
        let state = note_outcome(&pool, s.id, DeliveryStatus::Failed, "five failed")
            .await
            .unwrap()
            .unwrap();
        assert_eq!((state.consecutive_failures, state.paused_now), (n, false));
    }
    // A success before the fifth starts over.
    let reset = note_outcome(&pool, s.id, DeliveryStatus::Sent, "")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reset.consecutive_failures, 0);
    // A skip changes nothing.
    let skip = note_outcome(&pool, s.id, DeliveryStatus::Skipped, "")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(skip.consecutive_failures, 0);

    let mut last = None;
    for _ in 0..PAUSE_AFTER_FAILURES {
        last = note_outcome(&pool, s.id, DeliveryStatus::Failed, "five failed")
            .await
            .unwrap();
    }
    let last = last.unwrap();
    assert_eq!(last.consecutive_failures, PAUSE_AFTER_FAILURES);
    assert!(last.paused_now, "the fifth failure paused it");
    let stored = get(&pool, s.id).await.unwrap().unwrap();
    assert!(stored.paused);
    assert_eq!(stored.paused_reason.as_deref(), Some("five failed"));
    // A sixth failure (a "Send now") does not announce a second pause.
    let sixth = note_outcome(&pool, s.id, DeliveryStatus::Failed, "five failed")
        .await
        .unwrap()
        .unwrap();
    assert!(!sixth.paused_now);

    // Resuming clears the reason and the count and sets the next time.
    let when = OffsetDateTime::now_utc() + Duration::days(1);
    let resumed = set_paused(&pool, s.id, false, None, Some(when))
        .await
        .unwrap()
        .unwrap();
    assert!(!resumed.paused && resumed.paused_reason.is_none());
    assert_eq!(resumed.consecutive_failures, 0);
    assert!((resumed.next_run_at - when).abs() < Duration::milliseconds(5));
    assert!(
        note_outcome(&pool, Uuid::new_v4(), DeliveryStatus::Sent, "")
            .await
            .unwrap()
            .is_none()
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn deliveries_are_listed_newest_first_and_the_old_ones_are_pruned(
    pool: PgPool,
) -> sqlx::Result<()> {
    let owner = user(&pool, "owner").await;
    let a = create(&pool, &new_sub(owner, "a", OffsetDateTime::now_utc()))
        .await
        .unwrap();
    let b = create(&pool, &new_sub(owner, "b", OffsetDateTime::now_utc()))
        .await
        .unwrap();
    let now = OffsetDateTime::now_utc();
    let record = |sub: Uuid, ago: Duration, status: DeliveryStatus, reason: Option<&str>| {
        let pool = pool.clone();
        let reason = reason.map(str::to_owned);
        async move {
            record_delivery(
                &pool,
                &NewDelivery {
                    subscription_id: sub,
                    started_at: now - ago,
                    finished_at: now - ago + Duration::seconds(3),
                    status,
                    manual: false,
                    recipient_count: 2,
                    reason,
                },
            )
            .await
            .unwrap()
        }
    };
    record(a.id, Duration::days(100), DeliveryStatus::Sent, None).await;
    record(
        a.id,
        Duration::days(2),
        DeliveryStatus::Failed,
        Some("The PDF could not be made."),
    )
    .await;
    record(a.id, Duration::hours(1), DeliveryStatus::Sent, None).await;
    record(
        b.id,
        Duration::days(3),
        DeliveryStatus::Skipped,
        Some("Every chart was empty."),
    )
    .await;

    let listed = list_deliveries(&pool, a.id, 10).await.unwrap();
    assert_eq!(
        listed.iter().map(|d| d.status.as_str()).collect::<Vec<_>>(),
        ["sent", "failed", "sent"]
    );
    assert_eq!(list_deliveries(&pool, a.id, 2).await.unwrap().len(), 2);
    let last = last_deliveries(&pool, &[a.id, b.id]).await.unwrap();
    assert_eq!(last.len(), 2);
    assert_eq!(
        last.iter()
            .find(|d| d.subscription_id == b.id)
            .unwrap()
            .status,
        "skipped"
    );

    let removed = prune_deliveries(&pool, now - Duration::days(90))
        .await
        .unwrap();
    assert_eq!(removed, 1, "only the 100-day-old one went");
    assert_eq!(list_deliveries(&pool, a.id, 10).await.unwrap().len(), 2);

    // Deleting a subscription takes its recipients and deliveries along.
    assert!(delete(&pool, a.id).await.unwrap());
    assert!(!delete(&pool, a.id).await.unwrap());
    assert!(list_deliveries(&pool, a.id, 10).await.unwrap().is_empty());
    assert!(recipients(&pool, a.id).await.unwrap().is_empty());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn deleting_the_owner_removes_their_subscriptions(pool: PgPool) -> sqlx::Result<()> {
    let owner = user(&pool, "owner").await;
    let s = create(&pool, &new_sub(owner, "a", OffsetDateTime::now_utc()))
        .await
        .unwrap();
    sqlx::query("DELETE FROM app_user WHERE id = $1")
        .bind(owner)
        .execute(&pool)
        .await?;
    assert!(get(&pool, s.id).await.unwrap().is_none());
    Ok(())
}

/// RPT-1 review fix (SHOULD-FIX) R9: a retry brings the next time forward, never
/// back, and leaves a paused subscription alone.
#[sqlx::test(migrations = "../../migrations")]
async fn a_retry_brings_the_next_time_forward_and_never_pushes_it_back(
    pool: PgPool,
) -> sqlx::Result<()> {
    let owner = user(&pool, "owner").await;
    let now = OffsetDateTime::now_utc();
    let s = create(&pool, &new_sub(owner, "a", now + Duration::days(1)))
        .await
        .unwrap();
    assert!(
        retry_at(&pool, s.id, now + Duration::minutes(5))
            .await
            .unwrap()
    );
    let moved = get(&pool, s.id).await.unwrap().unwrap().next_run_at;
    assert!((moved - (now + Duration::minutes(5))).abs() < Duration::milliseconds(5));
    // A later time does not push it back.
    assert!(
        !retry_at(&pool, s.id, now + Duration::hours(2))
            .await
            .unwrap()
    );
    assert_eq!(get(&pool, s.id).await.unwrap().unwrap().next_run_at, moved);
    // A paused subscription stays where it is.
    set_paused(&pool, s.id, true, Some("x"), None)
        .await
        .unwrap();
    assert!(
        !retry_at(&pool, s.id, now + Duration::minutes(1))
            .await
            .unwrap()
    );
    Ok(())
}
