//! Integration tests for `lakehouse_store::connector_run` against a real
//! Postgres (`SRC-11` tasks 1; same setup as `tests/connectors.rs`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_test_support as _;

use lakehouse_store::StoreError;
use lakehouse_store::connector_run::{
    AttemptReport, EndReason, ErrorClass, LogEvent, RUN_HISTORY_LIMIT, RunStatus, RunTrigger,
    add_log_line, end_run, get_run_log, list_runs, purge_older_than, record_attempt, run_status,
    settle_run, start_run,
};
use sqlx::PgPool;
use time::{Duration, OffsetDateTime};

const PG: &str = "conn-pg-lakehouse";
const S3: &str = "conn-s3-warehouse";

fn failed(table: &str, attempt: i32, at: OffsetDateTime, wait: Option<i32>) -> AttemptReport<'_> {
    AttemptReport {
        table,
        attempt,
        started_at: at,
        ended_at: at + Duration::seconds(2),
        rows: 0,
        error_class: Some(ErrorClass::Unreachable),
        wait_seconds: wait,
    }
}

/// `SRC-11` D5/D7: a run with two attempts at one table keeps each attempt
/// and the wait between them, totals the rows of the attempt that
/// succeeded, and renders the log in the order things happened.
#[sqlx::test(migrations = "../../migrations")]
async fn a_run_with_two_attempts_keeps_each_attempt_and_its_totals(
    pool: PgPool,
) -> sqlx::Result<()> {
    let t0 = OffsetDateTime::now_utc() - Duration::minutes(10);
    assert!(
        start_run(&pool, PG, "run-a", RunTrigger::Schedule, None, t0)
            .await
            .unwrap()
    );
    assert!(
        record_attempt(
            &pool,
            PG,
            "run-a",
            &failed("public.orders", 1, t0, Some(60))
        )
        .await
        .unwrap()
    );
    let second = AttemptReport {
        table: "public.orders",
        attempt: 2,
        started_at: t0 + Duration::seconds(62),
        ended_at: t0 + Duration::seconds(65),
        rows: 120,
        error_class: None,
        wait_seconds: None,
    };
    assert!(record_attempt(&pool, PG, "run-a", &second).await.unwrap());
    assert!(
        end_run(
            &pool,
            PG,
            "run-a",
            RunStatus::Succeeded,
            EndReason::Completed,
            t0 + Duration::seconds(66)
        )
        .await
        .unwrap()
    );

    let runs = list_runs(&pool, PG, RUN_HISTORY_LIMIT).await.unwrap();
    assert_eq!(runs.len(), 1);
    let run = &runs[0];
    assert_eq!(
        (
            run.status.as_str(),
            run.trigger.as_str(),
            run.attempts,
            run.rows_loaded
        ),
        ("succeeded", "schedule", 2, 120)
    );
    assert_eq!(run.duration_seconds, Some(66));
    assert_eq!(run.end_reason.as_deref(), Some("completed"));

    let log = get_run_log(&pool, PG, "run-a").await.unwrap().unwrap();
    assert_eq!(log.attempts.len(), 2);
    assert_eq!(log.attempts[0].error_class.as_deref(), Some("unreachable"));
    assert_eq!(log.attempts[0].wait_seconds, Some(60));
    assert_eq!(log.attempts[1].result, "succeeded");
    let events: Vec<&str> = log.lines.iter().map(|l| l.event.as_str()).collect();
    assert_eq!(
        events,
        [
            "run_started",
            "attempt",
            "retry_wait",
            "attempt",
            "run_ended"
        ]
    );
    let text = log.to_text();
    assert_eq!(text.lines().count(), 1 + log.lines.len());
    assert!(
        text.contains("table public.orders, attempt 1: failed, the source could not be reached")
    );
    assert!(text.contains("table public.orders: next attempt in 1 min"));
    assert!(text.contains("table public.orders, attempt 2: loaded 120 rows in 3 s"));
    Ok(())
}

/// A report posted twice (the orchestrator did not see the answer) changes
/// nothing the second time, and the first report's facts stand.
#[sqlx::test(migrations = "../../migrations")]
async fn a_repeated_report_changes_nothing(pool: PgPool) -> sqlx::Result<()> {
    let t0 = OffsetDateTime::now_utc();
    assert!(
        start_run(&pool, PG, "run-r", RunTrigger::User, None, t0)
            .await
            .unwrap()
    );
    assert!(
        !start_run(&pool, PG, "run-r", RunTrigger::Schedule, None, t0)
            .await
            .unwrap()
    );
    let attempt = failed("t", 1, t0, None);
    assert!(record_attempt(&pool, PG, "run-r", &attempt).await.unwrap());
    assert!(!record_attempt(&pool, PG, "run-r", &attempt).await.unwrap());
    assert!(
        end_run(
            &pool,
            PG,
            "run-r",
            RunStatus::Failed,
            EndReason::TableFailed,
            t0
        )
        .await
        .unwrap()
    );
    assert!(
        !end_run(
            &pool,
            PG,
            "run-r",
            RunStatus::Succeeded,
            EndReason::Completed,
            t0
        )
        .await
        .unwrap()
    );
    let log = get_run_log(&pool, PG, "run-r").await.unwrap().unwrap();
    assert_eq!(log.run.trigger, "user");
    assert_eq!(log.run.status, "failed");
    assert_eq!(log.run.attempts, 1);
    assert_eq!(log.run.rows_loaded, 0, "a failed attempt adds no rows");
    assert_eq!(log.lines.len(), 3, "start, one attempt, end");
    Ok(())
}

/// A run belongs to one connector: another connector's report is refused and
/// its log is not readable through another connector's id.
#[sqlx::test(migrations = "../../migrations")]
async fn a_run_cannot_be_reported_or_read_through_another_connector(
    pool: PgPool,
) -> sqlx::Result<()> {
    let t0 = OffsetDateTime::now_utc();
    start_run(&pool, PG, "run-x", RunTrigger::Other, None, t0)
        .await
        .unwrap();
    let err = start_run(&pool, S3, "run-x", RunTrigger::Other, None, t0)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::Conflict), "{err:?}");
    let err = record_attempt(&pool, S3, "run-x", &failed("t", 1, t0, None))
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::NotFound), "{err:?}");
    let err = end_run(&pool, S3, "run-x", RunStatus::Failed, EndReason::Other, t0)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::NotFound), "{err:?}");
    assert!(get_run_log(&pool, S3, "run-x").await.unwrap().is_none());
    assert!(get_run_log(&pool, PG, "run-nope").await.unwrap().is_none());
    assert_eq!(
        list_runs(&pool, S3, 50).await.unwrap(),
        [] as [lakehouse_store::connector_run::RunSummary; 0]
    );
    assert_eq!(
        run_status(&pool, PG, "run-x").await.unwrap().as_deref(),
        Some("running")
    );
    assert_eq!(run_status(&pool, S3, "run-x").await.unwrap(), None);
    Ok(())
}

/// D7: the history is the newest 50 of THIS connector, however many runs
/// there are and whatever other connectors ran.
#[sqlx::test(migrations = "../../migrations")]
async fn the_history_is_the_newest_fifty_runs_of_the_connector(pool: PgPool) -> sqlx::Result<()> {
    let base = OffsetDateTime::now_utc() - Duration::hours(2);
    for n in 0..55 {
        start_run(
            &pool,
            PG,
            &format!("run-{n:02}"),
            RunTrigger::Schedule,
            None,
            base + Duration::minutes(n),
        )
        .await
        .unwrap();
    }
    for n in 0..3 {
        start_run(
            &pool,
            S3,
            &format!("other-{n}"),
            RunTrigger::User,
            None,
            base + Duration::minutes(n),
        )
        .await
        .unwrap();
    }
    let runs = list_runs(&pool, PG, RUN_HISTORY_LIMIT).await.unwrap();
    assert_eq!(runs.len(), 50);
    assert_eq!(runs[0].run_id, "run-54");
    assert_eq!(runs[49].run_id, "run-05", "the five oldest are not listed");
    assert!(runs.iter().all(|r| r.run_id.starts_with("run-")));
    assert_eq!(
        list_runs(&pool, S3, RUN_HISTORY_LIMIT).await.unwrap().len(),
        3
    );
    Ok(())
}

/// D5/D6: the daily job removes log lines and attempts older than the
/// cutoff, only those, and keeps the runs with their totals.
#[sqlx::test(migrations = "../../migrations")]
async fn retention_removes_old_lines_and_attempts_and_keeps_the_runs(
    pool: PgPool,
) -> sqlx::Result<()> {
    let now = OffsetDateTime::now_utc();
    let old = now - Duration::days(40);
    let recent = now - Duration::days(1);
    for (run, at) in [("run-old", old), ("run-new", recent)] {
        start_run(&pool, PG, run, RunTrigger::Schedule, None, at)
            .await
            .unwrap();
        let attempt = AttemptReport {
            table: "t",
            attempt: 1,
            started_at: at,
            ended_at: at + Duration::seconds(1),
            rows: 7,
            error_class: None,
            wait_seconds: None,
        };
        record_attempt(&pool, PG, run, &attempt).await.unwrap();
        end_run(
            &pool,
            PG,
            run,
            RunStatus::Succeeded,
            EndReason::Completed,
            at + Duration::seconds(2),
        )
        .await
        .unwrap();
    }
    let purged = purge_older_than(&pool, now - Duration::days(30))
        .await
        .unwrap();
    assert_eq!((purged.lines, purged.attempts), (3, 1));

    let old_log = get_run_log(&pool, PG, "run-old").await.unwrap().unwrap();
    assert!(old_log.lines.is_empty() && old_log.attempts.is_empty());
    assert_eq!((old_log.run.attempts, old_log.run.rows_loaded), (1, 7));
    let new_log = get_run_log(&pool, PG, "run-new").await.unwrap().unwrap();
    assert_eq!((new_log.lines.len(), new_log.attempts.len()), (3, 1));
    assert_eq!(list_runs(&pool, PG, 50).await.unwrap().len(), 2);
    Ok(())
}

/// A report that contradicts itself is refused and writes nothing; a log
/// event that needs a table or a count and has none is refused too.
#[sqlx::test(migrations = "../../migrations")]
async fn contradictory_reports_are_refused(pool: PgPool) -> sqlx::Result<()> {
    let t0 = OffsetDateTime::now_utc();
    start_run(&pool, PG, "run-c", RunTrigger::User, None, t0)
        .await
        .unwrap();
    let mut wait_on_success = failed("t", 1, t0, Some(60));
    wait_on_success.error_class = None;
    let zero = AttemptReport {
        attempt: 0,
        ..failed("t", 1, t0, None)
    };
    for report in [wait_on_success, zero] {
        let err = record_attempt(&pool, PG, "run-c", &report)
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::Validation(_)), "{err:?}");
    }
    let err = add_log_line(
        &pool,
        PG,
        "run-c",
        LogEvent::ColumnsLeftOut,
        Some("t"),
        None,
        t0,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, StoreError::Validation(_)), "{err:?}");
    let err = end_run(&pool, PG, "run-c", RunStatus::Running, EndReason::Other, t0)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::Validation(_)), "{err:?}");
    add_log_line(
        &pool,
        PG,
        "run-c",
        LogEvent::ColumnsLeftOut,
        Some("t"),
        Some(2),
        t0,
    )
    .await
    .unwrap();
    add_log_line(&pool, PG, "run-c", LogEvent::ReportGap, None, None, t0)
        .await
        .unwrap();
    let log = get_run_log(&pool, PG, "run-c").await.unwrap().unwrap();
    assert_eq!(
        log.attempts,
        [] as [lakehouse_store::connector_run::AttemptRecord; 0]
    );
    assert_eq!(log.lines.len(), 3);
    assert!(log.lines[1].text.contains("2 columns left out"));
    Ok(())
}

/// The sensor settles a run the orchestrator never closed, leaves a run that
/// already ended (here `paused`) alone, and invents nothing for a run it has
/// no record of.
#[sqlx::test(migrations = "../../migrations")]
async fn the_sensor_settles_only_a_run_still_running(pool: PgPool) -> sqlx::Result<()> {
    let t0 = OffsetDateTime::now_utc();
    start_run(&pool, PG, "run-open", RunTrigger::Schedule, None, t0)
        .await
        .unwrap();
    start_run(&pool, PG, "run-paused", RunTrigger::Other, None, t0)
        .await
        .unwrap();
    end_run(
        &pool,
        PG,
        "run-paused",
        RunStatus::Paused,
        EndReason::ConnectorPaused,
        t0,
    )
    .await
    .unwrap();
    settle_run(&pool, PG, "run-open", false, t0).await.unwrap();
    settle_run(&pool, PG, "run-paused", true, t0).await.unwrap();
    settle_run(&pool, PG, "run-unknown", true, t0)
        .await
        .unwrap();
    assert_eq!(
        run_status(&pool, PG, "run-open").await.unwrap().as_deref(),
        Some("failed")
    );
    assert_eq!(
        run_status(&pool, PG, "run-paused")
            .await
            .unwrap()
            .as_deref(),
        Some("paused")
    );
    assert_eq!(run_status(&pool, PG, "run-unknown").await.unwrap(), None);
    Ok(())
}

/// `SRC-11` review 1 BLOCKER 2: a refresh or clear names its table in the run
/// record (the run list says "Clear · public.orders", `SRC-10` decision 5);
/// any other run has none, and a table on another trigger is refused.
#[sqlx::test(migrations = "../../migrations")]
async fn a_refresh_or_clear_run_keeps_its_table_and_no_other_run_has_one(
    pool: PgPool,
) -> sqlx::Result<()> {
    let t0 = OffsetDateTime::now_utc();
    start_run(
        &pool,
        PG,
        "run-clear",
        RunTrigger::Clear,
        Some("public.orders"),
        t0,
    )
    .await
    .unwrap();
    start_run(
        &pool,
        PG,
        "run-sched",
        RunTrigger::Schedule,
        None,
        t0 + Duration::seconds(1),
    )
    .await
    .unwrap();
    let err = start_run(
        &pool,
        PG,
        "run-bad",
        RunTrigger::Schedule,
        Some("public.orders"),
        t0,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, StoreError::Validation(_)), "{err:?}");
    assert_eq!(run_status(&pool, PG, "run-bad").await.unwrap(), None);

    let runs = list_runs(&pool, PG, RUN_HISTORY_LIMIT).await.unwrap();
    assert_eq!(
        runs.iter()
            .map(|r| (r.run_id.as_str(), r.table.as_deref()))
            .collect::<Vec<_>>(),
        [("run-sched", None), ("run-clear", Some("public.orders"))]
    );
    let log = get_run_log(&pool, PG, "run-clear").await.unwrap().unwrap();
    assert_eq!(log.run.table.as_deref(), Some("public.orders"));
    Ok(())
}
