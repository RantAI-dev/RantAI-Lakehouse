//! Compute a schedule's next `Dagster`-UTC fire time from its cron
//! expression, server-side (`nextRunAt`, WS4 item G2). `Dagster` schedules
//! in this code location never set `execution_timezone` (confirmed:
//! `grep -rn "execution_timezone" dagster/dispar_orchestrate/*.py` returns
//! nothing) so `Dagster`'s own UTC default applies uniformly — this
//! function always evaluates in UTC, never the server's local timezone.

use croner::Cron;
use croner::errors::CronError;
use croner::parser::{CronParser, Seconds, Year};
use time::OffsetDateTime;

/// Parse `cron_expr` the way `croner` 2.x did, under `croner` 4.x.
///
/// `croner` 4 changed two parsing defaults that this module's callers rely
/// on (`~/.cargo/registry/src/.../croner-4.0.1/MIGRATION.md`, and the field
/// count in `CronParser::parse`), so both are pinned here instead of taking
/// `Cron::from_str`, which is `CronParser::new()`:
///
/// - seconds and year fields are refused (`Seconds::Disallowed`,
///   `Year::Disallowed`): 2.x took exactly five fields unless
///   `with_seconds_optional()` was called, which this module never did,
///   whereas the 4.x default accepts five to seven. A longer expression
///   stays unparsable rather than gaining a meaning (see
///   [`ingest_cron_problem`] for why a seconds field could never fire);
/// - `sloppy_ranges(true)`: 2.x read `5/10` as "from 5 to the maximum, every
///   10" and 4.x rejects it by default (OCPS strict steps). Stored schedules
///   and connector `scheduleCron` values written against 2.x keep parsing.
///   The same flag would also admit a bare step such as `/10` (read as
///   `*/10`), which 2.x refused. A schedule is user input and validation
///   must not widen silently (fail closed), so a list element that starts
///   with `/` is refused before `croner` sees it.
///
/// # Errors
///
/// [`CronError::InvalidPattern`] for a bare step; otherwise [`CronError`]
/// when `cron_expr` is not a valid five-field cron expression (or `@`
/// nickname) under the settings above.
fn parse_cron(cron_expr: &str) -> Result<Cron, CronError> {
    if cron_expr
        .split_whitespace()
        .flat_map(|field| field.split(','))
        .any(|element| element.starts_with('/'))
    {
        return Err(CronError::InvalidPattern(
            "a step needs a range or `*` before the `/`".to_owned(),
        ));
    }
    CronParser::builder()
        .seconds(Seconds::Disallowed)
        .year(Year::Disallowed)
        .sloppy_ranges(true)
        .build()
        .parse(cron_expr)
}

/// The next fire time strictly after `now`, or `None` when `cron_expr` is
/// not a 5-field cron expression (e.g. `"manual"`, `dagster_pipeline_row`'s
/// own `schedule_label` fallback), fails to parse, or `croner` itself
/// cannot compute a next occurrence (an exhausted iteration, which cannot
/// happen for a real recurring 5-field expression but is not assumed away
/// here). A `nextRunAt` that cannot be computed is `None` — never a guess,
/// never "now", never the epoch.
#[must_use]
pub fn next_run_at(cron_expr: &str, now: OffsetDateTime) -> Option<OffsetDateTime> {
    let cron = parse_cron(cron_expr).ok()?;
    let now_chrono = chrono::DateTime::from_timestamp(now.unix_timestamp(), 0)?;
    let next = cron.find_next_occurrence(&now_chrono, false).ok()?;
    OffsetDateTime::from_unix_timestamp(next.timestamp()).ok()
}

/// Why `cron_expr` cannot be a connector's ingest schedule, or `None`
/// when it can. Five fields only: `ingest_schedule_sensor`
/// (`dagster/dispar_orchestrate/ingest_factory.py`) checks schedules a
/// minute at a time, so a seconds field could never fire as written.
#[must_use]
pub fn ingest_cron_problem(cron_expr: &str) -> Option<String> {
    if cron_expr.split_whitespace().count() != 5 {
        return Some(format!(
            "scheduleCron {cron_expr:?} must have five fields: minute hour day-of-month month day-of-week"
        ));
    }
    parse_cron(cron_expr)
        .err()
        .map(|err| format!("scheduleCron {cron_expr:?} is not a valid cron expression: {err}"))
}

/// Whether `cron_expr` fires in `(after, until]`, evaluated in UTC like
/// [`next_run_at`] — the same `croner` evaluation, so the connector the
/// schedule sensor launches is the one the console said would run then.
/// `false` for an expression that does not parse.
#[must_use]
pub fn fires_between(cron_expr: &str, after: OffsetDateTime, until: OffsetDateTime) -> bool {
    next_run_at(cron_expr, after).is_some_and(|next| next <= until)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn next_run_at_computes_the_next_utc_fire_time_after_now() {
        let now = time::macros::datetime!(2026 - 09 - 11 10:00:00 UTC);
        let next = next_run_at("0 3 * * *", now).expect("a daily cron has a next occurrence");
        assert_eq!(next.hour(), 3);
        assert!(next > now);
    }

    #[test]
    fn next_run_at_none_for_manual_or_malformed_cron() {
        assert!(next_run_at("manual", time::OffsetDateTime::now_utc()).is_none());
        assert!(next_run_at("not a cron", time::OffsetDateTime::now_utc()).is_none());
    }

    // `croner` 4 changed two defaults; these pin the 2.x behaviour that
    // `parse_cron` keeps (see its doc comment).
    #[test]
    fn next_run_at_refuses_a_seconds_or_year_field_as_croner_2_did() {
        let now = time::macros::datetime!(2026 - 09 - 11 10:00:00 UTC);
        assert!(next_run_at("0 0 3 * * *", now).is_none());
        assert!(next_run_at("0 0 3 * * * 2030", now).is_none());
    }

    #[test]
    fn next_run_at_keeps_the_start_step_shorthand_croner_2_accepted() {
        let now = time::macros::datetime!(2026 - 09 - 11 10:00:00 UTC);
        // "from minute 5 to the end of the hour, every 10": 10:05 comes first.
        let next = next_run_at("5/10 * * * *", now).expect("2.x accepted the shorthand");
        assert_eq!((next.hour(), next.minute()), (10, 5));
        assert_eq!(ingest_cron_problem("0/15 * * * *"), None);
    }

    #[test]
    fn a_bare_step_is_refused_as_croner_2_did() {
        let now = time::macros::datetime!(2026 - 09 - 11 10:00:00 UTC);
        for expr in [
            "/10 * * * *",
            "*/5 /10 * * *",
            "1,/10 * * * *",
            "* * * * /2",
        ] {
            assert!(next_run_at(expr, now).is_none(), "{expr}");
            assert!(ingest_cron_problem(expr).is_some(), "{expr}");
        }
        assert!(matches!(
            parse_cron("/10 * * * *"),
            Err(CronError::InvalidPattern(_))
        ));
    }

    #[test]
    fn ingest_cron_problem_accepts_five_fields_only() {
        assert_eq!(ingest_cron_problem("0 2 * * *"), None);
        assert_eq!(ingest_cron_problem("*/15 8-17 * * 1-5"), None);
        // A seconds field, a nickname and garbage are all refused.
        assert!(ingest_cron_problem("0 0 2 * * *").is_some());
        assert!(ingest_cron_problem("@daily").is_some());
        assert!(ingest_cron_problem("0 25 * * *").is_some());
        assert!(ingest_cron_problem("a b c d e").is_some());
    }

    #[test]
    fn fires_between_includes_the_window_end_but_not_its_start() {
        let at_two = time::macros::datetime!(2026 - 09 - 30 02:00:00 UTC);
        let minute = time::Duration::minutes(1);
        assert!(fires_between("0 2 * * *", at_two - minute, at_two));
        // The window before already had it: never twice.
        assert!(!fires_between("0 2 * * *", at_two, at_two + minute));
        // A wide window (a caught-up gap) still finds it.
        assert!(fires_between(
            "0 2 * * *",
            at_two - time::Duration::hours(1),
            at_two + minute
        ));
        assert!(!fires_between("0 3 * * *", at_two - minute, at_two));
        assert!(!fires_between("not a cron", at_two - minute, at_two));
    }

    #[test]
    fn next_run_at_none_for_empty_cron() {
        // `dagster_pipeline_row`'s own `schedule_label` never emits an
        // empty string, but `next_run_at`'s own contract must not panic
        // or guess on one -- `None`, same as any other unparseable input.
        assert!(next_run_at("", time::OffsetDateTime::now_utc()).is_none());
    }
}
