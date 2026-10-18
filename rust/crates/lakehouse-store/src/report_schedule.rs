//! When a report subscription is next due (`RPT-1`, plan decision D2, feature
//! decision 4). Pure functions, no database: the runner and the routes both
//! call [`Schedule::next_run_after`], so a schedule cannot mean one thing when
//! it is saved and another when it runs.
//!
//! # Why this module owns a time zone database
//!
//! The report time zone is chosen from the engine's zone list (`BI-9`), but the
//! runner must turn "Mondays at 08:00 in Asia/Jakarta" into an instant without
//! asking the engine on every tick, and a daylight-saving change has to be
//! tested in a unit test. `jiff` carries the IANA database itself
//! (`tzdb-bundle-always`), so the answer does not depend on the container image
//! having `tzdata`.
//!
//! # The rules at a daylight-saving change
//!
//! - A time that does not exist (the clock jumps from 02:00 to 03:00, a daily
//!   02:30) runs at the first instant after the gap, 03:00: the next valid
//!   minute. It never runs twice and never gets skipped for that day.
//! - A time that happens twice (the clock goes back from 03:00 to 02:00, a daily
//!   02:30) runs at its first occurrence only: a repeated time runs once.
//! - An hourly schedule is the exception on purpose: it names a minute of the
//!   hour, and the repeated hour is a real second hour that has passed, so it
//!   runs in both. An hour that does not exist has no run.
//!
//! # A missed run happens once (feature decision 8)
//!
//! The next time is always worked out from the instant the runner claims the
//! row, never from the time the row was due. An API that was down for three days
//! claims a daily subscription once, and the next run is tomorrow.

use jiff::civil::Date;
use jiff::tz::{AmbiguousOffset, TimeZone};
use jiff::{Span, Timestamp};
use time::OffsetDateTime;

/// How far ahead a search looks before it gives up. A monthly schedule is
/// found within 31 days; the rest is slack for a zone whose calendar jumps.
const SEARCH_DAYS: i64 = 400;

/// Why a next time could not be worked out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleError {
    /// The zone name is not in the time zone database.
    UnknownZone,
    /// The schedule has a field out of range (a minute above 59, a day above
    /// 28 that is not "last").
    Invalid,
    /// No run time was found inside the search window.
    NoRunTime,
}

impl ScheduleError {
    /// The sentence a person is told.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::UnknownZone => "That time zone is not known.",
            Self::Invalid => "That schedule is not valid.",
            Self::NoRunTime => "The next run time could not be worked out.",
        }
    }
}

impl std::fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for ScheduleError {}

/// The day of the month a monthly schedule runs on. Days 29 to 31 do not exist
/// because they would skip months; "the last day" is its own choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonthDay {
    /// A day from 1 to 28.
    Day(u8),
    /// The last day of the month, whatever its length.
    Last,
}

/// One schedule: what the person chose, before the time zone is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Schedule {
    /// Every hour at `minute` past.
    Hourly {
        /// 0 to 59.
        minute: u8,
    },
    /// Every day at `hour:minute`.
    Daily {
        /// 0 to 23.
        hour: u8,
        /// 0 to 59.
        minute: u8,
    },
    /// Every week on `weekday` (ISO: 1 = Monday to 7 = Sunday) at `hour:minute`.
    Weekly {
        /// 1 to 7.
        weekday: u8,
        /// 0 to 23.
        hour: u8,
        /// 0 to 59.
        minute: u8,
    },
    /// Every month on `day` at `hour:minute`.
    Monthly {
        /// The day of the month.
        day: MonthDay,
        /// 0 to 23.
        hour: u8,
        /// 0 to 59.
        minute: u8,
    },
}

/// The columns of `report_subscription` that hold a schedule, in the shape the
/// table stores them (migration `0088`): `kind`, `minute`, `hour`, `weekday`,
/// `month_day` (`-1` is the last day).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduleParts {
    /// `hourly`, `daily`, `weekly` or `monthly`.
    pub kind: &'static str,
    /// 0 to 59.
    pub minute: i16,
    /// `None` for hourly.
    pub hour: Option<i16>,
    /// `Some` for weekly only.
    pub weekday: Option<i16>,
    /// `Some` for monthly only; `-1` is the last day.
    pub month_day: Option<i16>,
}

impl Schedule {
    /// Check the ranges the table's `CHECK`s also enforce.
    ///
    /// # Errors
    ///
    /// [`ScheduleError::Invalid`] for a minute above 59, an hour above 23, a
    /// weekday outside 1 to 7 or a monthly day outside 1 to 28.
    pub fn validate(self) -> Result<(), ScheduleError> {
        let ok = match self {
            Self::Hourly { minute } => minute < 60,
            Self::Daily { hour, minute } => hour < 24 && minute < 60,
            Self::Weekly {
                weekday,
                hour,
                minute,
            } => (1..=7).contains(&weekday) && hour < 24 && minute < 60,
            Self::Monthly { day, hour, minute } => {
                let day_ok = match day {
                    MonthDay::Day(d) => (1..=28).contains(&d),
                    MonthDay::Last => true,
                };
                day_ok && hour < 24 && minute < 60
            }
        };
        if ok {
            Ok(())
        } else {
            Err(ScheduleError::Invalid)
        }
    }

    /// The stored columns of this schedule.
    #[must_use]
    pub fn to_parts(self) -> ScheduleParts {
        match self {
            Self::Hourly { minute } => ScheduleParts {
                kind: "hourly",
                minute: i16::from(minute),
                hour: None,
                weekday: None,
                month_day: None,
            },
            Self::Daily { hour, minute } => ScheduleParts {
                kind: "daily",
                minute: i16::from(minute),
                hour: Some(i16::from(hour)),
                weekday: None,
                month_day: None,
            },
            Self::Weekly {
                weekday,
                hour,
                minute,
            } => ScheduleParts {
                kind: "weekly",
                minute: i16::from(minute),
                hour: Some(i16::from(hour)),
                weekday: Some(i16::from(weekday)),
                month_day: None,
            },
            Self::Monthly { day, hour, minute } => ScheduleParts {
                kind: "monthly",
                minute: i16::from(minute),
                hour: Some(i16::from(hour)),
                weekday: None,
                month_day: Some(match day {
                    MonthDay::Day(d) => i16::from(d),
                    MonthDay::Last => -1,
                }),
            },
        }
    }

    /// The schedule a row stores.
    ///
    /// # Errors
    ///
    /// [`ScheduleError::Invalid`] for an unknown kind, a field the kind needs
    /// that is missing, or a value out of range.
    pub fn from_parts(
        kind: &str,
        minute: i16,
        hour: Option<i16>,
        weekday: Option<i16>,
        month_day: Option<i16>,
    ) -> Result<Self, ScheduleError> {
        let narrow = |v: i16| u8::try_from(v).map_err(|_| ScheduleError::Invalid);
        let minute = narrow(minute)?;
        let need_hour = || hour.ok_or(ScheduleError::Invalid).and_then(narrow);
        let schedule = match kind {
            "hourly" => Self::Hourly { minute },
            "daily" => Self::Daily {
                hour: need_hour()?,
                minute,
            },
            "weekly" => Self::Weekly {
                weekday: weekday.ok_or(ScheduleError::Invalid).and_then(narrow)?,
                hour: need_hour()?,
                minute,
            },
            "monthly" => Self::Monthly {
                day: match month_day.ok_or(ScheduleError::Invalid)? {
                    -1 => MonthDay::Last,
                    d => MonthDay::Day(narrow(d)?),
                },
                hour: need_hour()?,
                minute,
            },
            _ => return Err(ScheduleError::Invalid),
        };
        schedule.validate()?;
        Ok(schedule)
    }

    /// The first run time strictly after `now`, in `zone` (an IANA name).
    ///
    /// # Errors
    ///
    /// [`ScheduleError::UnknownZone`] for a zone the database does not have,
    /// [`ScheduleError::Invalid`] for a field out of range,
    /// [`ScheduleError::NoRunTime`] when nothing was found in the search
    /// window (not expected for any real zone).
    pub fn next_run_after(
        self,
        zone: &str,
        now: OffsetDateTime,
    ) -> Result<OffsetDateTime, ScheduleError> {
        self.validate()?;
        let tz = TimeZone::get(zone).map_err(|_| ScheduleError::UnknownZone)?;
        // Whole seconds: a run time has none, so a `now` of 08:00:00.5 for a
        // schedule at 08:00 is already past it.
        let now_ts =
            Timestamp::from_second(now.unix_timestamp()).map_err(|_| ScheduleError::Invalid)?;
        let next = match self {
            Self::Hourly { minute } => next_hourly(&tz, now_ts, minute)?,
            Self::Daily { hour, minute } => next_civil(&tz, now_ts, |_| true, hour, minute)?,
            Self::Weekly {
                weekday,
                hour,
                minute,
            } => next_civil(
                &tz,
                now_ts,
                |d| d.weekday().to_monday_one_offset() == weekday.cast_signed(),
                hour,
                minute,
            )?,
            Self::Monthly { day, hour, minute } => next_civil(
                &tz,
                now_ts,
                |d| match day {
                    MonthDay::Day(n) => d.day() == n.cast_signed(),
                    MonthDay::Last => d.last_of_month() == d,
                },
                hour,
                minute,
            )?,
        };
        OffsetDateTime::from_unix_timestamp(next.as_second()).map_err(|_| ScheduleError::NoRunTime)
    }
}

/// Whether `zone` is in the time zone database. The routes use it to refuse an
/// unknown zone before anything is saved.
#[must_use]
pub fn zone_is_known(zone: &str) -> bool {
    TimeZone::get(zone).is_ok()
}

/// `now` as a `YYYY-MM-DD` date on the wall clock of `zone`, for the subject of
/// a report ("<dashboard> report, <date in the subscription's zone>", plan D5).
/// `None` for a zone the database does not have.
#[must_use]
pub fn local_date(zone: &str, now: OffsetDateTime) -> Option<String> {
    let tz = TimeZone::get(zone).ok()?;
    let at = Timestamp::from_second(now.unix_timestamp()).ok()?;
    let date = at.to_zoned(tz).date();
    Some(format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        date.month(),
        date.day()
    ))
}

/// Hourly: walk the minutes after `now` until the zone's wall clock shows the
/// chosen minute. Walking instants (not wall-clock times) is what makes a
/// repeated hour run twice and a missing hour not run, and it is right in zones
/// whose offset is not a whole hour. At most 60 steps for any real zone.
fn next_hourly(tz: &TimeZone, now: Timestamp, minute: u8) -> Result<Timestamp, ScheduleError> {
    let first = (now.as_second().div_euclid(60) + 1) * 60;
    for step in 0..180 {
        let ts = Timestamp::from_second(first + step * 60).map_err(|_| ScheduleError::NoRunTime)?;
        if ts.to_zoned(tz.clone()).minute() == minute.cast_signed() {
            return Ok(ts);
        }
    }
    Err(ScheduleError::NoRunTime)
}

/// Daily, weekly and monthly: walk the zone's calendar days from the day `now`
/// falls on, take the days `on_day` accepts, and turn `hour:minute` of that day
/// into an instant ([`resolve`]). The first instant after `now` wins.
fn next_civil(
    tz: &TimeZone,
    now: Timestamp,
    on_day: impl Fn(Date) -> bool,
    hour: u8,
    minute: u8,
) -> Result<Timestamp, ScheduleError> {
    let today = now.to_zoned(tz.clone()).date();
    for ahead in 0..=SEARCH_DAYS {
        let Ok(day) = today.checked_add(Span::new().days(ahead)) else {
            break;
        };
        if !on_day(day) {
            continue;
        }
        let at = day.at(hour.cast_signed(), minute.cast_signed(), 0, 0);
        let ts = resolve(tz, at)?;
        if ts > now {
            return Ok(ts);
        }
    }
    Err(ScheduleError::NoRunTime)
}

/// The instant of a wall-clock time. Unambiguous: that instant. A repeated
/// time: its first occurrence. A time inside a gap: the instant the gap ends.
fn resolve(tz: &TimeZone, at: jiff::civil::DateTime) -> Result<Timestamp, ScheduleError> {
    let ambiguous = tz.to_ambiguous_timestamp(at);
    match ambiguous.offset() {
        AmbiguousOffset::Unambiguous { .. } | AmbiguousOffset::Fold { .. } => {
            ambiguous.earlier().map_err(|_| ScheduleError::NoRunTime)
        }
        AmbiguousOffset::Gap { after, .. } => {
            // `at` read with the offset after the gap is an instant before the
            // gap ends; the first transition after it is the gap's end.
            let before_gap_end = after
                .to_timestamp(at)
                .map_err(|_| ScheduleError::NoRunTime)?;
            tz.following(before_gap_end)
                .next()
                .map(|t| t.timestamp())
                .ok_or(ScheduleError::NoRunTime)
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// `YYYY-MM-DDTHH:MM:SSZ` as an `OffsetDateTime`.
    fn utc(text: &str) -> OffsetDateTime {
        OffsetDateTime::parse(text, &time::format_description::well_known::Rfc3339).unwrap()
    }

    fn next(schedule: Schedule, zone: &str, now: &str) -> String {
        let at = schedule.next_run_after(zone, utc(now)).unwrap();
        at.format(&time::format_description::well_known::Rfc3339)
            .unwrap()
    }

    #[test]
    fn an_hourly_schedule_runs_at_its_minute_in_the_next_hour_that_has_it() {
        let s = Schedule::Hourly { minute: 30 };
        assert_eq!(
            next(s, "UTC", "2026-10-19T08:10:00Z"),
            "2026-10-19T08:30:00Z"
        );
        // Exactly on the minute is already past it.
        assert_eq!(
            next(s, "UTC", "2026-10-19T08:30:00Z"),
            "2026-10-19T09:30:00Z"
        );
    }

    #[test]
    fn an_hourly_schedule_reads_the_minute_on_the_zones_own_clock() {
        // India is UTC+5:30, so local :15 is UTC :45.
        let s = Schedule::Hourly { minute: 15 };
        assert_eq!(
            next(s, "Asia/Kolkata", "2026-01-01T00:00:00Z"),
            "2026-01-01T00:45:00Z"
        );
    }

    #[test]
    fn a_daily_schedule_uses_the_zone_and_rolls_to_the_next_day() {
        // Jakarta is UTC+7 with no daylight saving.
        let s = Schedule::Daily { hour: 8, minute: 0 };
        assert_eq!(
            next(s, "Asia/Jakarta", "2026-10-19T00:00:00Z"),
            "2026-10-19T01:00:00Z"
        );
        assert_eq!(
            next(s, "Asia/Jakarta", "2026-10-19T01:00:00Z"),
            "2026-10-20T01:00:00Z"
        );
    }

    #[test]
    fn a_weekly_schedule_finds_its_weekday() {
        // 2026-10-19 is a Monday.
        let s = Schedule::Weekly {
            weekday: 1,
            hour: 8,
            minute: 0,
        };
        assert_eq!(
            next(s, "Asia/Jakarta", "2026-10-19T00:00:00Z"),
            "2026-10-19T01:00:00Z"
        );
        // Just after Monday 08:00 local: a week later.
        assert_eq!(
            next(s, "Asia/Jakarta", "2026-10-19T01:00:01Z"),
            "2026-10-26T01:00:00Z"
        );
        // Sunday is 7.
        let sunday = Schedule::Weekly {
            weekday: 7,
            hour: 9,
            minute: 15,
        };
        assert_eq!(
            next(sunday, "UTC", "2026-10-19T00:00:00Z"),
            "2026-10-25T09:15:00Z"
        );
    }

    #[test]
    fn the_week_of_a_late_evening_follows_the_zones_date_not_utcs() {
        // 2026-10-18 20:00Z is already Monday 03:00 in Jakarta.
        let s = Schedule::Weekly {
            weekday: 1,
            hour: 8,
            minute: 0,
        };
        assert_eq!(
            next(s, "Asia/Jakarta", "2026-10-18T20:00:00Z"),
            "2026-10-19T01:00:00Z"
        );
    }

    #[test]
    fn a_monthly_schedule_on_the_last_day_follows_the_month_length() {
        let s = Schedule::Monthly {
            day: MonthDay::Last,
            hour: 23,
            minute: 30,
        };
        assert_eq!(
            next(s, "UTC", "2026-02-10T00:00:00Z"),
            "2026-02-28T23:30:00Z"
        );
        // 2028 is a leap year.
        assert_eq!(
            next(s, "UTC", "2028-02-10T00:00:00Z"),
            "2028-02-29T23:30:00Z"
        );
        assert_eq!(
            next(s, "UTC", "2026-04-30T23:30:00Z"),
            "2026-05-31T23:30:00Z"
        );
    }

    #[test]
    fn a_monthly_schedule_on_day_28_never_skips_a_month() {
        let s = Schedule::Monthly {
            day: MonthDay::Day(28),
            hour: 6,
            minute: 0,
        };
        assert_eq!(
            next(s, "UTC", "2026-02-28T06:00:00Z"),
            "2026-03-28T06:00:00Z"
        );
        assert_eq!(
            next(s, "UTC", "2026-12-28T06:00:00Z"),
            "2027-01-28T06:00:00Z"
        );
    }

    #[test]
    fn a_time_that_does_not_exist_runs_at_the_next_valid_minute() {
        // Europe/Amsterdam springs forward on 2026-03-29: 02:00 CET becomes
        // 03:00 CEST, so 02:30 does not exist that day. The run is at 03:00
        // local, which is 01:00Z.
        let s = Schedule::Daily {
            hour: 2,
            minute: 30,
        };
        assert_eq!(
            next(s, "Europe/Amsterdam", "2026-03-28T12:00:00Z"),
            "2026-03-29T01:00:00Z"
        );
        // And the day after is the ordinary 02:30 CEST = 00:30Z.
        assert_eq!(
            next(s, "Europe/Amsterdam", "2026-03-29T01:00:00Z"),
            "2026-03-30T00:30:00Z"
        );
    }

    #[test]
    fn a_time_that_happens_twice_runs_once() {
        // Europe/Amsterdam goes back on 2026-10-25: 03:00 CEST becomes 02:00
        // CET, so 02:30 happens at 00:30Z and again at 01:30Z. Only the first
        // is a run.
        let s = Schedule::Daily {
            hour: 2,
            minute: 30,
        };
        assert_eq!(
            next(s, "Europe/Amsterdam", "2026-10-24T12:00:00Z"),
            "2026-10-25T00:30:00Z"
        );
        // After the first, the next is the day after (02:30 CET = 01:30Z), not
        // the repeated 01:30Z of the same day.
        assert_eq!(
            next(s, "Europe/Amsterdam", "2026-10-25T00:30:00Z"),
            "2026-10-26T01:30:00Z"
        );
    }

    #[test]
    fn an_hourly_schedule_runs_in_both_passes_of_a_repeated_hour() {
        let s = Schedule::Hourly { minute: 30 };
        assert_eq!(
            next(s, "Europe/Amsterdam", "2026-10-25T00:30:00Z"),
            "2026-10-25T01:30:00Z"
        );
    }

    #[test]
    fn a_weekly_schedule_keeps_its_local_time_across_the_change() {
        // Monday 08:00 in Amsterdam is 07:00Z in winter and 06:00Z in summer.
        let s = Schedule::Weekly {
            weekday: 1,
            hour: 8,
            minute: 0,
        };
        assert_eq!(
            next(s, "Europe/Amsterdam", "2026-10-19T12:00:00Z"),
            "2026-10-26T07:00:00Z"
        );
        assert_eq!(
            next(s, "Europe/Amsterdam", "2026-03-23T12:00:00Z"),
            "2026-03-30T06:00:00Z"
        );
    }

    #[test]
    fn a_run_that_was_missed_for_days_is_worked_out_from_now_so_it_runs_once() {
        // The row was due on the 10th; the API comes back on the 14th. The
        // next time is the 15th, not a catch-up for each day in between.
        let s = Schedule::Daily { hour: 8, minute: 0 };
        assert_eq!(
            next(s, "UTC", "2026-10-14T09:00:00Z"),
            "2026-10-15T08:00:00Z"
        );
    }

    #[test]
    fn an_unknown_zone_and_an_out_of_range_field_are_refused() {
        let s = Schedule::Daily { hour: 8, minute: 0 };
        assert_eq!(
            s.next_run_after("Mars/Olympus", utc("2026-10-14T09:00:00Z")),
            Err(ScheduleError::UnknownZone)
        );
        assert_eq!(
            Schedule::Daily {
                hour: 24,
                minute: 0
            }
            .validate(),
            Err(ScheduleError::Invalid)
        );
        assert_eq!(
            Schedule::Monthly {
                day: MonthDay::Day(29),
                hour: 0,
                minute: 0
            }
            .validate(),
            Err(ScheduleError::Invalid)
        );
        assert!(zone_is_known("Asia/Jakarta"));
        assert!(!zone_is_known("Not/AZone"));
    }

    #[test]
    fn the_report_date_is_the_date_on_the_zones_clock() {
        // 20:00Z on the 18th is already the 19th in Jakarta (UTC+7).
        let now = utc("2026-10-18T20:00:00Z");
        assert_eq!(
            local_date("Asia/Jakarta", now).as_deref(),
            Some("2026-10-19")
        );
        assert_eq!(local_date("UTC", now).as_deref(), Some("2026-10-18"));
        assert_eq!(local_date("Mars/Olympus", now), None);
    }

    #[test]
    fn a_schedule_round_trips_through_the_stored_columns() {
        let all = [
            Schedule::Hourly { minute: 5 },
            Schedule::Daily { hour: 8, minute: 0 },
            Schedule::Weekly {
                weekday: 3,
                hour: 17,
                minute: 45,
            },
            Schedule::Monthly {
                day: MonthDay::Day(1),
                hour: 0,
                minute: 0,
            },
            Schedule::Monthly {
                day: MonthDay::Last,
                hour: 23,
                minute: 59,
            },
        ];
        for s in all {
            let p = s.to_parts();
            assert_eq!(
                Schedule::from_parts(p.kind, p.minute, p.hour, p.weekday, p.month_day),
                Ok(s)
            );
        }
        // A weekly row without its weekday is not a schedule.
        assert_eq!(
            Schedule::from_parts("weekly", 0, Some(8), None, None),
            Err(ScheduleError::Invalid)
        );
        assert_eq!(
            Schedule::from_parts("yearly", 0, None, None, None),
            Err(ScheduleError::Invalid)
        );
    }
}
