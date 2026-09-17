//! Compute a schedule's next `Dagster`-UTC fire time from its cron
//! expression, server-side (`nextRunAt`, WS4 item G2). `Dagster` schedules
//! in this code location never set `execution_timezone` (confirmed:
//! `grep -rn "execution_timezone" dagster/dispar_orchestrate/*.py` returns
//! nothing) so `Dagster`'s own UTC default applies uniformly — this
//! function always evaluates in UTC, never the server's local timezone.

use croner::Cron;
use time::OffsetDateTime;

/// The next fire time strictly after `now`, or `None` when `cron_expr` is
/// not a 5-field cron expression (e.g. `"manual"`, `dagster_pipeline_row`'s
/// own `schedule_label` fallback), fails to parse, or `croner` itself
/// cannot compute a next occurrence (an exhausted iteration, which cannot
/// happen for a real recurring 5-field expression but is not assumed away
/// here). A `nextRunAt` that cannot be computed is `None` — never a guess,
/// never "now", never the epoch.
#[must_use]
pub fn next_run_at(cron_expr: &str, now: OffsetDateTime) -> Option<OffsetDateTime> {
    // `croner` 2.x's real public API, read from its own source
    // (`~/.cargo/registry/src/.../croner-2.2.0/src/lib.rs`) after the
    // plan doc's own hedge ("the exact return type ... was not
    // independently verified against the crate's source") turned out to
    // matter: `Cron` is a BUILDER, not a value `FromStr` parses in one
    // step — `Cron::from_str` (its actual `impl`) just calls `Cron::new`
    // and always returns `Ok`, doing NO validation; the real parse/
    // validation step is the separate `&mut self -> Result<Cron,
    // CronError>` method `.parse()`. Using `FromStr`/`?` here (as the
    // plan doc's snippet did) would silently accept "not a cron" and
    // only fail later, if at all, inside `find_next_occurrence` — this
    // body calls `Cron::new(cron_expr).parse()` instead, the combination
    // that actually rejects a malformed pattern. There is also no
    // `chrono` CARGO FEATURE (`rust/Cargo.toml` no longer requests one —
    // `chrono` is croner's one, non-optional, always-on datetime
    // backend) and no generic `CronDateTime` trait in this version;
    // `find_next_occurrence<Tz: chrono::TimeZone>(&self, start:
    // &chrono::DateTime<Tz>, inclusive: bool) -> Result<DateTime<Tz>,
    // CronError>` is chrono-specific already, matching this function's
    // own `chrono::DateTime<Utc>` round trip below directly.
    let cron = Cron::new(cron_expr).parse().ok()?;
    let now_chrono = chrono::DateTime::from_timestamp(now.unix_timestamp(), 0)?;
    let next = cron.find_next_occurrence(&now_chrono, false).ok()?;
    OffsetDateTime::from_unix_timestamp(next.timestamp()).ok()
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

    #[test]
    fn next_run_at_none_for_empty_cron() {
        // `dagster_pipeline_row`'s own `schedule_label` never emits an
        // empty string, but `next_run_at`'s own contract must not panic
        // or guess on one -- `None`, same as any other unparseable input.
        assert!(next_run_at("", time::OffsetDateTime::now_utc()).is_none());
    }
}
