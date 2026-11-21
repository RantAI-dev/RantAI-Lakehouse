//! `lakehouse_store::alert_state` against a real Postgres (`BI-19` part C, D11):
//! a rule without a row, `since` moving only when the state changes, and a
//! failed check leaving the state and the last value alone.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_store::alert_state::{
    DigestTurn, DigestTurnError, clear_digest_clock, digest_clock, digest_is_due, digest_turn, get,
    give_back_digest, move_digest_clock, record_check, record_failed_check,
};
use lakehouse_store::report_schedule::Schedule;
use lakehouse_test_support as _;
use sqlx::PgPool;
use time::OffsetDateTime;

fn at(secs: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_800_000_000 + secs).unwrap()
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_rule_without_a_row_has_no_state(pool: PgPool) {
    assert_eq!(get(&pool, "al_none").await.unwrap(), None);
}

#[sqlx::test(migrations = "../../migrations")]
async fn since_moves_only_when_the_state_changes(pool: PgPool) {
    record_check(&pool, "al_1", "firing", Some(150.0), None, at(0))
        .await
        .unwrap();
    record_check(&pool, "al_1", "firing", Some(160.0), None, at(900))
        .await
        .unwrap();
    let row = get(&pool, "al_1").await.unwrap().unwrap();
    assert_eq!(row.state, "firing");
    assert_eq!(row.since, at(0));
    assert_eq!(row.last_checked_at, at(900));
    assert_eq!(row.last_value, Some(160.0));
    record_check(&pool, "al_1", "ok", Some(40.0), None, at(1800))
        .await
        .unwrap();
    assert_eq!(get(&pool, "al_1").await.unwrap().unwrap().since, at(1800));
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_failed_check_moves_the_time_and_reason_and_nothing_else(pool: PgPool) {
    record_check(&pool, "al_1", "firing", Some(150.0), None, at(0))
        .await
        .unwrap();
    record_failed_check(&pool, "al_1", "the value could not be read.", at(900))
        .await
        .unwrap();
    let row = get(&pool, "al_1").await.unwrap().unwrap();
    assert_eq!(row.state, "firing");
    assert_eq!(row.last_value, Some(150.0));
    assert_eq!(row.last_checked_at, at(900));
    assert_eq!(row.reason.as_deref(), Some("the value could not be read."));
    // A rule never checked before gets an `ok` row.
    record_failed_check(&pool, "al_2", "x", at(0))
        .await
        .unwrap();
    assert_eq!(get(&pool, "al_2").await.unwrap().unwrap().state, "ok");
}

// ── Digests (`BI-65`) ───────────────────────────────────────────────────

/// Saturday 2026-11-21 10:00 UTC, the tick that first sees a digest.
const SAT: i64 = 1_795_255_200;
/// Monday 2026-11-23 in Asia/Jakarta (UTC+7): 07:59, 08:00 and 08:15.
const MON_0759: i64 = 1_795_395_540;
const MON_0800: i64 = 1_795_395_600;
const MON_0815: i64 = 1_795_396_500;
const NEXT_MON_0800: i64 = 1_796_000_400;

fn utc(secs: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(secs).unwrap()
}

fn weekly_monday_8() -> Schedule {
    Schedule::Weekly {
        weekday: 1,
        hour: 8,
        minute: 0,
    }
}

const JKT: &str = "Asia/Jakarta";

#[test]
fn a_digest_is_due_once_the_first_run_after_its_clock_has_passed() {
    // The first Monday 08:00 (Jakarta) after Saturday is 2026-11-23 01:00 UTC.
    assert!(!digest_is_due(weekly_monday_8(), JKT, utc(SAT), utc(MON_0759)).unwrap());
    assert!(digest_is_due(weekly_monday_8(), JKT, utc(SAT), utc(MON_0800)).unwrap());
    // Counted from a clock at the run itself, the next one is a week away.
    assert!(!digest_is_due(weekly_monday_8(), JKT, utc(MON_0800), utc(MON_0815)).unwrap());
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_weekly_digest_is_not_sent_before_its_time_and_sent_once_after_it(pool: PgPool) {
    let weekly = weekly_monday_8();
    // The first tick that sees it only starts the clock.
    assert_eq!(
        digest_turn(&pool, "al_d", weekly, JKT, utc(SAT))
            .await
            .unwrap(),
        DigestTurn::StartedClock
    );
    assert_eq!(digest_clock(&pool, "al_d").await.unwrap(), Some(utc(SAT)));
    // One minute before Monday 08:00 in Jakarta: not yet.
    assert_eq!(
        digest_turn(&pool, "al_d", weekly, JKT, utc(MON_0759))
            .await
            .unwrap(),
        DigestTurn::NotYet
    );
    // At 08:00 the send is claimed once...
    assert_eq!(
        digest_turn(&pool, "al_d", weekly, JKT, utc(MON_0800))
            .await
            .unwrap(),
        DigestTurn::Claimed { previous: utc(SAT) }
    );
    // ...and the next 15-minute tick does not send again.
    assert_eq!(
        digest_turn(&pool, "al_d", weekly, JKT, utc(MON_0815))
            .await
            .unwrap(),
        DigestTurn::NotYet
    );
    // A week later it is due again.
    assert!(matches!(
        digest_turn(&pool, "al_d", weekly, JKT, utc(NEXT_MON_0800))
            .await
            .unwrap(),
        DigestTurn::Claimed { .. }
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_digest_with_no_schedule_of_its_own_is_due_once_a_day(pool: PgPool) {
    // The default for a digest saved before schedules: every day at 08:00.
    let daily = Schedule::Daily { hour: 8, minute: 0 };
    digest_turn(&pool, "al_old", daily, JKT, utc(SAT))
        .await
        .unwrap();
    let mut sent = 0;
    // Every 15 minutes from Saturday 10:00 UTC for 48 hours.
    for tick in 1..=192 {
        if matches!(
            digest_turn(&pool, "al_old", daily, JKT, utc(SAT + tick * 900))
                .await
                .unwrap(),
            DigestTurn::Claimed { .. }
        ) {
            sent += 1;
        }
    }
    assert_eq!(sent, 2, "one send a day, not one per tick");
}

#[sqlx::test(migrations = "../../migrations")]
async fn two_runners_cannot_both_claim_the_same_send(pool: PgPool) {
    digest_turn(&pool, "al_d", weekly_monday_8(), JKT, utc(SAT))
        .await
        .unwrap();
    assert!(
        move_digest_clock(&pool, "al_d", Some(utc(SAT)), utc(MON_0800))
            .await
            .unwrap()
    );
    // The second runner read the same clock, so it loses.
    assert!(
        !move_digest_clock(&pool, "al_d", Some(utc(SAT)), utc(MON_0800))
            .await
            .unwrap()
    );
    // A clock that was never started is claimed by one caller only.
    assert!(
        move_digest_clock(&pool, "al_new", None, utc(SAT))
            .await
            .unwrap()
    );
    assert!(
        !move_digest_clock(&pool, "al_new", None, utc(SAT))
            .await
            .unwrap()
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_send_given_back_is_due_again_on_the_next_tick(pool: PgPool) {
    let weekly = weekly_monday_8();
    digest_turn(&pool, "al_d", weekly, JKT, utc(SAT))
        .await
        .unwrap();
    let DigestTurn::Claimed { previous } = digest_turn(&pool, "al_d", weekly, JKT, utc(MON_0800))
        .await
        .unwrap()
    else {
        panic!("the digest should have been claimed");
    };
    give_back_digest(&pool, "al_d", utc(MON_0800), previous)
        .await
        .unwrap();
    assert!(matches!(
        digest_turn(&pool, "al_d", weekly, JKT, utc(MON_0815))
            .await
            .unwrap(),
        DigestTurn::Claimed { .. }
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_digests_row_is_ok_and_an_unknown_zone_is_refused(pool: PgPool) {
    digest_turn(&pool, "al_d", weekly_monday_8(), JKT, utc(SAT))
        .await
        .unwrap();
    assert_eq!(get(&pool, "al_d").await.unwrap().unwrap().state, "ok");
    let err = digest_turn(&pool, "al_d", weekly_monday_8(), "Mars/Base", utc(MON_0800))
        .await
        .unwrap_err();
    assert!(matches!(err, DigestTurnError::Schedule(_)));
}

#[sqlx::test(migrations = "../../migrations")]
async fn editing_a_digests_schedule_starts_its_clock_over(pool: PgPool) {
    let weekly = weekly_monday_8();
    digest_turn(&pool, "al_d", weekly, JKT, utc(SAT))
        .await
        .unwrap();
    clear_digest_clock(&pool, "al_d").await.unwrap();
    assert_eq!(digest_clock(&pool, "al_d").await.unwrap(), None);
    // Seen afresh at Monday 08:15: the clock starts, nothing is sent for the
    // 08:00 that passed under the old schedule.
    assert_eq!(
        digest_turn(&pool, "al_d", weekly, JKT, utc(MON_0815))
            .await
            .unwrap(),
        DigestTurn::StartedClock
    );
}
