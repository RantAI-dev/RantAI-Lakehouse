//! Time grain (`BI-9`, with `AI-3`): grouping a date or timestamp column by
//! day, week, month, quarter, year (and minute or hour), or by a part of the
//! date (hour of day, day of week, ...), in the deployment's report time
//! zone.
//!
//! # What lives here
//!
//! - [`Grain`], the thirteen values a chart may carry, and the rules for which
//!   chart kinds and column kinds take which ([`Grain::validate`]).
//! - [`TimeContext`], the two deployment-wide settings (the report time zone
//!   and the first day of the week) that every bucket expression and every
//!   relative date filter reads. The builder never reads the clock or the
//!   zone from anywhere else, so a chart and a date filter cannot disagree
//!   about where a month begins.
//! - [`Grain::bucket_expr`], the one place the `ClickHouse` expressions are
//!   written, and [`resolve`], which decides what a chart's grain is *right
//!   now* once the dashboard's own switch and the column's current type are
//!   known.
//!
//! # Why `date_trunc` and not `toStartOfMonth`
//!
//! Measured on `ClickHouse` 26.8 (`docs/superpowers/plans/2026-10-10-bi-9-time-grain.md`
//! section 7): `toStartOfMonth`, `toStartOfWeek`, `toStartOfQuarter`,
//! `toStartOfYear` and `toDate` return a `Date`, which cannot hold a
//! `Date32`/`DateTime64` value before 1970 or after 2149; such a value is
//! silently clipped (`toStartOfMonth(toDate32('1960-05-15'))` is
//! `1970-01-01`). `date_trunc('month', x)` keeps `Date32` for a `Date32` and a
//! `DateTime64` column, and `Date` otherwise, so it is used for week and
//! coarser. A day bucket of a timestamp is `toDate32`, for the same reason.
//!
//! # Why the bucket is typed, not text
//!
//! The expression returns a `Date`/`DateTime`, which the JSON output prints
//! as `YYYY-MM-DD` / `YYYY-MM-DD HH:MM:SS`. Grouping on a `toString` would
//! merge the repeated local hour of a daylight-saving change; grouping on the
//! typed value keeps the two instants apart (they print alike, which is
//! honest: the wall clock did repeat).
//!
//! # Safety of the SQL
//!
//! The only caller-supplied strings are the time zone name and a column name.
//! A zone name is built only through [`TimeZoneName::new`] (a strict
//! character pattern) and is printed through
//! [`lakehouse_core::ident::SqlLiteral`]; the API additionally checks it
//! against the engine's own list before it is saved. A column is an
//! [`lakehouse_core::ident::Ident`]. Everything else is a fixed function name.

use lakehouse_core::ident::{Ident, SqlLiteral};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::filters::ColumnKind;
use crate::specs::ChartKind;

/// The report time zone a deployment starts with.
pub const DEFAULT_TIME_ZONE: &str = "Asia/Jakarta";

/// Longest accepted time zone name. The longest name in the IANA database is
/// well under this.
const MAX_ZONE_LEN: usize = 64;

/// Most buckets a grained chart may ask for.
pub const MAX_GRAIN_LIMIT: u32 = 1000;

/// The first day of the week.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WeekStart {
    /// ISO weeks; the default.
    #[default]
    Monday,
    /// Sunday-first weeks.
    Sunday,
}

impl WeekStart {
    /// `"monday"` or `"sunday"`, the value stored and sent.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Monday => "monday",
            Self::Sunday => "sunday",
        }
    }

    /// The exact words `monday` and `sunday`; anything else is `None`.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "monday" => Some(Self::Monday),
            "sunday" => Some(Self::Sunday),
            _ => None,
        }
    }

    /// The `mode` argument of `toStartOfWeek`: 1 starts on Monday, 0 on
    /// Sunday.
    fn start_of_week_mode(self) -> u8 {
        match self {
            Self::Monday => 1,
            Self::Sunday => 0,
        }
    }

    /// The `mode` argument of `toWeek` that numbers weeks 1 to 53 with this
    /// first day: 3 is ISO 8601 (Monday; week 1 holds the year's first
    /// Thursday), 6 is the Sunday-first counterpart (week 1 holds at least
    /// four days of the year). Measured around 1 January for 2020 to 2027.
    fn week_of_year_mode(self) -> u8 {
        match self {
            Self::Monday => 3,
            Self::Sunday => 6,
        }
    }
}

/// An IANA time zone name that cannot break out of a string literal and has
/// the shape of one (`Asia/Jakarta`, `UTC`, `Etc/GMT+5`). The shape is a
/// second line of defence behind [`SqlLiteral`]; whether the engine knows the
/// name is the API's check (`routes::settings`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeZoneName(String);

impl TimeZoneName {
    /// `Some` when `raw` is 1 to 64 characters of `A-Za-z0-9_+-` with `/`
    /// between non-empty segments, starting with a letter.
    #[must_use]
    pub fn new(raw: &str) -> Option<Self> {
        let well_formed = !raw.is_empty()
            && raw.len() <= MAX_ZONE_LEN
            && raw.starts_with(|c: char| c.is_ascii_alphabetic())
            && raw
                .split('/')
                .all(|seg| !seg.is_empty() && seg.bytes().all(zone_byte));
        well_formed.then(|| Self(raw.to_owned()))
    }

    /// The name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn zone_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'+' | b'-')
}

/// Why a [`TimeContext`] could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidTimeZone;

/// The report time zone and the first day of the week: what "today",
/// "this month" and a bucket mean for the whole deployment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeContext {
    zone: TimeZoneName,
    week_start: WeekStart,
}

impl Default for TimeContext {
    /// `Asia/Jakarta`, Monday: what a deployment that never saved the
    /// settings gets.
    fn default() -> Self {
        Self {
            zone: TimeZoneName(DEFAULT_TIME_ZONE.to_owned()),
            week_start: WeekStart::Monday,
        }
    }
}

impl TimeContext {
    /// A context for `zone` (checked for shape only) and `week_start`.
    ///
    /// # Errors
    ///
    /// [`InvalidTimeZone`] when `zone` is not shaped like a zone name.
    pub fn new(zone: &str, week_start: WeekStart) -> Result<Self, InvalidTimeZone> {
        Ok(Self {
            zone: TimeZoneName::new(zone).ok_or(InvalidTimeZone)?,
            week_start,
        })
    }

    /// The zone name.
    #[must_use]
    pub fn zone(&self) -> &str {
        self.zone.as_str()
    }

    /// The first day of the week.
    #[must_use]
    pub fn week_start(&self) -> WeekStart {
        self.week_start
    }

    fn zone_literal(&self) -> SqlLiteral {
        SqlLiteral::from(self.zone.as_str())
    }

    /// "Today" in the report zone, as a `Date`.
    #[must_use]
    pub fn today_expr(&self) -> String {
        format!("toDate(now({}))", self.zone_literal())
    }

    /// The start of the current week in the report zone.
    #[must_use]
    pub fn week_start_expr(&self) -> String {
        format!(
            "toStartOfWeek({}, {})",
            self.today_expr(),
            self.week_start.start_of_week_mode()
        )
    }

    /// `column` as a calendar day in the report zone. A `Date` is never
    /// shifted; a timestamp is converted to the zone first. `Date32` so a
    /// range that starts in 1900 compares without clipping.
    #[must_use]
    pub fn day_expr(&self, column: &Ident, kind: ColumnKind) -> String {
        if kind == ColumnKind::DateTime {
            format!("toDate32({})", self.in_zone(column))
        } else {
            column.to_string()
        }
    }

    fn in_zone(&self, column: &Ident) -> String {
        self.in_zone_expr(&column.to_string())
    }

    /// `expr` (a timestamp expression) converted to the report zone. `expr`
    /// must already be SQL the caller built from validated parts.
    #[must_use]
    pub fn in_zone_expr(&self, expr: &str) -> String {
        format!("toTimeZone({expr}, {})", self.zone_literal())
    }

    /// The current instant in the report zone, as a `DateTime` (`BI-8`
    /// `Now()`).
    #[must_use]
    pub fn now_expr(&self) -> String {
        format!("now({})", self.zone_literal())
    }
}

/// How a date or timestamp column is grouped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Grain {
    /// Start of the minute (timestamp columns).
    Minute,
    /// Start of the hour (timestamp columns).
    Hour,
    /// The calendar day.
    Day,
    /// Start of the week, per the first-day setting.
    Week,
    /// First day of the month.
    Month,
    /// First day of the quarter.
    Quarter,
    /// First day of the year.
    Year,
    /// 0 to 23 (timestamp columns).
    HourOfDay,
    /// 1 (Monday) to 7 (Sunday), shown from the first day of the week.
    DayOfWeek,
    /// 1 to 31.
    DayOfMonth,
    /// 1 to 53, numbered by the first-day setting.
    WeekOfYear,
    /// 1 to 12.
    MonthOfYear,
    /// 1 to 4.
    QuarterOfYear,
}

/// Every grain, in the order the console offers them.
pub const ALL_GRAINS: [Grain; 13] = [
    Grain::Minute,
    Grain::Hour,
    Grain::Day,
    Grain::Week,
    Grain::Month,
    Grain::Quarter,
    Grain::Year,
    Grain::HourOfDay,
    Grain::DayOfWeek,
    Grain::DayOfMonth,
    Grain::WeekOfYear,
    Grain::MonthOfYear,
    Grain::QuarterOfYear,
];

impl Grain {
    /// The wire name (`"hour_of_day"`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Minute => "minute",
            Self::Hour => "hour",
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
            Self::Quarter => "quarter",
            Self::Year => "year",
            Self::HourOfDay => "hour_of_day",
            Self::DayOfWeek => "day_of_week",
            Self::DayOfMonth => "day_of_month",
            Self::WeekOfYear => "week_of_year",
            Self::MonthOfYear => "month_of_year",
            Self::QuarterOfYear => "quarter_of_year",
        }
    }

    /// One of the thirteen exact names; anything else is `None`.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        ALL_GRAINS.into_iter().find(|g| g.as_str() == raw)
    }

    /// A truncation (a bucket with a start) as opposed to a part of the date
    /// (a number).
    #[must_use]
    pub fn is_truncation(self) -> bool {
        matches!(
            self,
            Self::Minute
                | Self::Hour
                | Self::Day
                | Self::Week
                | Self::Month
                | Self::Quarter
                | Self::Year
        )
    }

    /// Needs a timestamp column: a `Date` has no hour or minute.
    #[must_use]
    pub fn needs_time(self) -> bool {
        matches!(self, Self::Minute | Self::Hour | Self::HourOfDay)
    }

    /// Whether a chart of `kind` can carry a grain at all, and which.
    fn kind_takes(self, kind: ChartKind) -> bool {
        match kind {
            ChartKind::Calendar => self == Self::Day,
            ChartKind::Bar
            | ChartKind::Hbar
            | ChartKind::Line
            | ChartKind::Area
            | ChartKind::Stacked
            | ChartKind::Combo
            | ChartKind::Waterfall
            | ChartKind::Heatmap
            | ChartKind::Pie
            | ChartKind::Rose
            | ChartKind::Funnel
            | ChartKind::Treemap
            | ChartKind::Radar
            | ChartKind::Pivot => true,
            _ => false,
        }
    }

    /// Whether this grain applies to a `kind` chart whose dimension column is
    /// of `column`. `None` is a column the relation does not have.
    #[must_use]
    pub fn supports(self, kind: ChartKind, column: Option<ColumnKind>) -> bool {
        match column {
            Some(ColumnKind::DateTime) => self.kind_takes(kind),
            Some(ColumnKind::Date) => !self.needs_time() && self.kind_takes(kind),
            _ => false,
        }
    }

    /// The reason a grain is refused for this chart, in plain words, or
    /// `Ok(())`. Called when a chart is saved.
    ///
    /// # Errors
    ///
    /// The message to show the person who chose the grain.
    pub fn validate(self, kind: ChartKind, column: Option<ColumnKind>) -> Result<(), String> {
        if kind == ChartKind::Calendar && self != Self::Day {
            return Err("a calendar chart can only group by day.".to_owned());
        }
        if !self.kind_takes(kind) {
            return Err("this chart type cannot group dates.".to_owned());
        }
        match column {
            Some(ColumnKind::Date) if self.needs_time() => Err(format!(
                "{} needs a timestamp column; this column has no time.",
                self.label()
            )),
            Some(ColumnKind::Date | ColumnKind::DateTime) => Ok(()),
            _ => Err("a grain needs a date or timestamp dimension.".to_owned()),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::HourOfDay => "hour of day",
            other => other.as_str(),
        }
    }

    /// The expression for `column` of `kind` in the report zone, or `None`
    /// for a combination [`Self::supports`] would refuse (a `Date` has no
    /// hour; a text column has no date).
    ///
    /// A truncation is a `Date` or a `DateTime` (printed by the engine as
    /// `YYYY-MM-DD` or `YYYY-MM-DD HH:MM:SS` in the report zone); a part is a
    /// small integer.
    #[must_use]
    pub fn bucket_expr(
        self,
        column: &Ident,
        kind: ColumnKind,
        ctx: &TimeContext,
    ) -> Option<String> {
        self.bucket_expr_over(&column.to_string(), kind, ctx)
    }

    /// [`Self::bucket_expr`] over any date or timestamp expression `value`
    /// (`BI-8`: `DateTrunc` in a formula uses the same buckets a chart's
    /// grain does). `value` is SQL the caller built from validated parts.
    #[must_use]
    pub fn bucket_expr_over(
        self,
        value: &str,
        kind: ColumnKind,
        ctx: &TimeContext,
    ) -> Option<String> {
        let x = match kind {
            ColumnKind::DateTime => ctx.in_zone_expr(value),
            ColumnKind::Date if !self.needs_time() => value.to_owned(),
            _ => return None,
        };
        Some(match self {
            Self::Minute => format!("date_trunc('minute', {x})"),
            Self::Hour => format!("date_trunc('hour', {x})"),
            // A `Date` is already a day. A timestamp becomes a `Date32` so a
            // value outside 1970 to 2149 is not clipped (module doc).
            Self::Day if kind == ColumnKind::DateTime => format!("toDate32({x})"),
            Self::Day => x,
            Self::Week => match ctx.week_start {
                WeekStart::Monday => format!("date_trunc('week', {x})"),
                // `date_trunc('week', ..)` is Monday-first: shift the day
                // forward, truncate, shift back.
                WeekStart::Sunday => {
                    format!("date_trunc('week', {x} + toIntervalDay(1)) - toIntervalDay(1)")
                }
            },
            Self::Month => format!("date_trunc('month', {x})"),
            Self::Quarter => format!("date_trunc('quarter', {x})"),
            Self::Year => format!("date_trunc('year', {x})"),
            Self::HourOfDay => format!("toHour({x})"),
            Self::DayOfWeek => format!("toDayOfWeek({x})"),
            Self::DayOfMonth => format!("toDayOfMonth({x})"),
            Self::WeekOfYear => format!("toWeek({x}, {})", ctx.week_start.week_of_year_mode()),
            Self::MonthOfYear => format!("toMonth({x})"),
            Self::QuarterOfYear => format!("toQuarter({x})"),
        })
    }

    /// The expression to `ORDER BY` for a bucket column named `bucket`: the
    /// bucket itself, except day of week, which is `1` (Monday) to `7`
    /// (Sunday) and so sorts Sunday last; with a Sunday-first week the key is
    /// `bucket % 7`, which puts Sunday (7) first.
    #[must_use]
    pub fn order_key(self, bucket: &str, week_start: WeekStart) -> String {
        if self == Self::DayOfWeek && week_start == WeekStart::Sunday {
            format!("{bucket} % 7")
        } else {
            bucket.to_owned()
        }
    }

    /// `toString(<bucket>) = '<value>'`: the rows of one clicked bucket,
    /// compared the way the engine prints the bucket (the value the tile
    /// carries). `None` when the grain does not apply to the column.
    #[must_use]
    pub fn records_predicate(
        self,
        column: &Ident,
        kind: ColumnKind,
        ctx: &TimeContext,
        value: &str,
    ) -> Option<String> {
        let expr = self.bucket_expr(column, kind, ctx)?;
        Some(format!("toString({expr}) = {}", SqlLiteral::from(value)))
    }
}

/// What a chart's grain is right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Resolution {
    /// The grain to build with, if any.
    pub effective: Option<Grain>,
    /// A dashboard switch this chart could not take; the chart kept its own
    /// and the tile says so.
    pub skipped: Option<Grain>,
}

/// Decide a chart's grain: its own (`def.grain`, kept only while the column
/// still fits it), replaced by the dashboard's `requested` truncation when
/// the chart's own is a truncation and the new one fits. A chart with a part,
/// or without a grain, ignores the switch.
#[must_use]
pub fn resolve(
    own: Option<&str>,
    kind: ChartKind,
    column: Option<ColumnKind>,
    requested: Option<Grain>,
) -> Resolution {
    let Some(own) = own.and_then(Grain::parse) else {
        return Resolution::default();
    };
    if !own.supports(kind, column) {
        return Resolution::default();
    }
    match requested {
        Some(req) if own.is_truncation() && req.is_truncation() && req != own => {
            if req.supports(kind, column) {
                Resolution {
                    effective: Some(req),
                    skipped: None,
                }
            } else {
                Resolution {
                    effective: Some(own),
                    skipped: Some(req),
                }
            }
        }
        _ => Resolution {
            effective: Some(own),
            skipped: None,
        },
    }
}

/// The limit a grained chart is built with: 1 to [`MAX_GRAIN_LIMIT`]
/// (default 20; 366 for a calendar), where other charts keep 1 to 100.
#[must_use]
pub fn clamp_limit(kind: ChartKind, requested: Option<u32>) -> u32 {
    let default = if kind == ChartKind::Calendar { 366 } else { 20 };
    requested.unwrap_or(default).clamp(1, MAX_GRAIN_LIMIT)
}

/// Whether a grained chart keeps the latest buckets when more exist than its
/// limit: true when it is ordered by bucket (`order` is `none`). The query
/// then asks for one bucket more than the limit so the cut can be seen
/// ([`trim_to_latest`]).
#[must_use]
pub fn keeps_latest(order: &str) -> bool {
    order == "none"
}

/// More than any part of the date can have (hours 24, weekdays 7, days of a
/// month 31, weeks of a year 53 or 54, months 12, quarters 4).
pub const PART_RANGE_LIMIT: u32 = 64;

/// BI-9 review fix (BLOCKER) R1: whether the query keeps the latest buckets.
/// Only a truncation ordered by bucket does; a part of the date (`hour_of_day`
/// and the like) has a fixed range, where "the latest" would drop hours 0 to 3.
#[must_use]
pub fn cuts_to_latest(grain: Grain, order: &str) -> bool {
    grain.is_truncation() && keeps_latest(order)
}

/// BI-9 review fix (BLOCKER) R1: the limit a grained query is built with. A
/// part ordered by bucket returns its whole range whatever limit was sent
/// ([`PART_RANGE_LIMIT`] can never cut); everything else keeps `limit`, a part
/// with a value order included (top N).
#[must_use]
pub fn effective_limit(grain: Grain, order: &str, limit: u32) -> u32 {
    if !grain.is_truncation() && keeps_latest(order) {
        PART_RANGE_LIMIT
    } else {
        limit
    }
}

/// Drop the surplus bucket of a query built with `limit + 1` buckets and say
/// whether a cut happened. `rows` are in ascending bucket order, `x` is the
/// bucket column. When more than `limit` distinct buckets came back, the
/// empty (`NULL`) bucket goes first, then the oldest ones.
pub fn trim_to_latest(rows: &mut Vec<Map<String, Value>>, x: &str, limit: usize) -> bool {
    let mut distinct: Vec<Value> = Vec::new();
    for row in rows.iter() {
        let v = row.get(x).cloned().unwrap_or(Value::Null);
        if !distinct.contains(&v) {
            distinct.push(v);
        }
    }
    let Some(excess) = distinct.len().checked_sub(limit).filter(|n| *n > 0) else {
        return false;
    };
    let mut drop: Vec<Value> = Vec::with_capacity(excess);
    if excess > 0 && distinct.contains(&Value::Null) {
        drop.push(Value::Null);
    }
    for v in &distinct {
        if drop.len() >= excess {
            break;
        }
        if *v != Value::Null {
            drop.push(v.clone());
        }
    }
    rows.retain(|row| !drop.contains(row.get(x).unwrap_or(&Value::Null)));
    true
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::json;

    use super::*;

    fn id(name: &str) -> Ident {
        Ident::new(name).unwrap()
    }

    fn ctx(week_start: WeekStart) -> TimeContext {
        TimeContext::new("Asia/Jakarta", week_start).unwrap()
    }

    #[test]
    fn the_thirteen_names_round_trip_and_nothing_else_parses() {
        assert_eq!(ALL_GRAINS.len(), 13);
        for g in ALL_GRAINS {
            assert_eq!(Grain::parse(g.as_str()), Some(g));
            let json = serde_json::to_string(&g).unwrap();
            assert_eq!(json, format!("\"{}\"", g.as_str()));
        }
        for bad in [
            "",
            "Day",
            "days",
            "fortnight",
            "hour of day",
            "day;--",
            "week ",
        ] {
            assert_eq!(Grain::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_zone_name_must_have_the_shape_of_one() {
        for ok in [
            "UTC",
            "Asia/Jakarta",
            "America/Argentina/Buenos_Aires",
            "Etc/GMT+5",
            "Etc/GMT-14",
        ] {
            assert!(TimeZoneName::new(ok).is_some(), "{ok}");
        }
        let too_long = "A".repeat(65);
        for bad in [
            "",
            "/UTC",
            "UTC/",
            "Asia//Jakarta",
            "Asia/Jakarta'",
            "Asia/Jakarta' OR 1=1 --",
            "Asia/Jak arta",
            "1Asia",
            "Asia\\Jakarta",
            "Asia/Jakarta\n",
            "Zürich",
            &too_long,
        ] {
            assert!(TimeZoneName::new(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn every_truncation_on_a_date_column() {
        let c = id("d");
        let ctx = ctx(WeekStart::Monday);
        let sql = |g: Grain| g.bucket_expr(&c, ColumnKind::Date, &ctx);
        assert_eq!(sql(Grain::Day).as_deref(), Some("d"));
        assert_eq!(sql(Grain::Week).as_deref(), Some("date_trunc('week', d)"));
        assert_eq!(sql(Grain::Month).as_deref(), Some("date_trunc('month', d)"));
        assert_eq!(
            sql(Grain::Quarter).as_deref(),
            Some("date_trunc('quarter', d)")
        );
        assert_eq!(sql(Grain::Year).as_deref(), Some("date_trunc('year', d)"));
        assert_eq!(sql(Grain::Minute), None, "a Date has no minute");
        assert_eq!(sql(Grain::Hour), None, "a Date has no hour");
        assert_eq!(sql(Grain::HourOfDay), None, "a Date has no hour");
    }

    #[test]
    fn every_value_on_a_timestamp_column_is_converted_to_the_report_zone_first() {
        let c = id("ts");
        let ctx = ctx(WeekStart::Monday);
        let z = "toTimeZone(ts, 'Asia/Jakarta')";
        let sql = |g: Grain| g.bucket_expr(&c, ColumnKind::DateTime, &ctx).unwrap();
        assert_eq!(sql(Grain::Minute), format!("date_trunc('minute', {z})"));
        assert_eq!(sql(Grain::Hour), format!("date_trunc('hour', {z})"));
        assert_eq!(sql(Grain::Day), format!("toDate32({z})"));
        assert_eq!(sql(Grain::Week), format!("date_trunc('week', {z})"));
        assert_eq!(sql(Grain::Month), format!("date_trunc('month', {z})"));
        assert_eq!(sql(Grain::Quarter), format!("date_trunc('quarter', {z})"));
        assert_eq!(sql(Grain::Year), format!("date_trunc('year', {z})"));
        assert_eq!(sql(Grain::HourOfDay), format!("toHour({z})"));
        assert_eq!(sql(Grain::DayOfWeek), format!("toDayOfWeek({z})"));
        assert_eq!(sql(Grain::DayOfMonth), format!("toDayOfMonth({z})"));
        assert_eq!(sql(Grain::WeekOfYear), format!("toWeek({z}, 3)"));
        assert_eq!(sql(Grain::MonthOfYear), format!("toMonth({z})"));
        assert_eq!(sql(Grain::QuarterOfYear), format!("toQuarter({z})"));
    }

    #[test]
    fn parts_on_a_date_column_are_not_shifted() {
        let c = id("d");
        let ctx = ctx(WeekStart::Monday);
        let sql = |g: Grain| g.bucket_expr(&c, ColumnKind::Date, &ctx).unwrap();
        assert_eq!(sql(Grain::DayOfWeek), "toDayOfWeek(d)");
        assert_eq!(sql(Grain::DayOfMonth), "toDayOfMonth(d)");
        assert_eq!(sql(Grain::WeekOfYear), "toWeek(d, 3)");
        assert_eq!(sql(Grain::MonthOfYear), "toMonth(d)");
        assert_eq!(sql(Grain::QuarterOfYear), "toQuarter(d)");
    }

    #[test]
    fn a_sunday_first_week_changes_the_week_bucket_and_the_week_number() {
        let c = id("d");
        let sun = ctx(WeekStart::Sunday);
        assert_eq!(
            Grain::Week.bucket_expr(&c, ColumnKind::Date, &sun).unwrap(),
            "date_trunc('week', d + toIntervalDay(1)) - toIntervalDay(1)"
        );
        assert_eq!(
            Grain::WeekOfYear
                .bucket_expr(&c, ColumnKind::Date, &sun)
                .unwrap(),
            "toWeek(d, 6)"
        );
        // Day of week stays 1 (Monday) to 7 (Sunday); only its order moves.
        assert_eq!(
            Grain::DayOfWeek
                .bucket_expr(&c, ColumnKind::Date, &sun)
                .unwrap(),
            "toDayOfWeek(d)"
        );
        assert_eq!(Grain::DayOfWeek.order_key("d", WeekStart::Sunday), "d % 7");
        assert_eq!(Grain::DayOfWeek.order_key("d", WeekStart::Monday), "d");
        assert_eq!(Grain::Month.order_key("d", WeekStart::Sunday), "d");
    }

    #[test]
    fn a_zone_is_printed_as_a_literal_and_the_context_reads_it_everywhere() {
        let ctx = TimeContext::new("Europe/Berlin", WeekStart::Sunday).unwrap();
        assert_eq!(ctx.today_expr(), "toDate(now('Europe/Berlin'))");
        assert_eq!(
            ctx.week_start_expr(),
            "toStartOfWeek(toDate(now('Europe/Berlin')), 0)"
        );
        assert_eq!(
            ctx.day_expr(&id("ts"), ColumnKind::DateTime),
            "toDate32(toTimeZone(ts, 'Europe/Berlin'))"
        );
        assert_eq!(ctx.day_expr(&id("d"), ColumnKind::Date), "d");
        assert!(TimeContext::new("Europe/Berlin'; DROP", WeekStart::Monday).is_err());
        assert_eq!(TimeContext::default().zone(), DEFAULT_TIME_ZONE);
    }

    #[test]
    fn a_clicked_bucket_is_compared_as_the_text_the_engine_prints() {
        let ctx = ctx(WeekStart::Monday);
        assert_eq!(
            Grain::Month
                .records_predicate(&id("d"), ColumnKind::Date, &ctx, "2026-03-01")
                .unwrap(),
            "toString(date_trunc('month', d)) = '2026-03-01'"
        );
        // A value cannot leave its literal.
        let p = Grain::Month
            .records_predicate(&id("d"), ColumnKind::Date, &ctx, "x' OR 1=1 --")
            .unwrap();
        assert!(p.ends_with("= 'x'' OR 1=1 --'"), "{p}");
        assert_eq!(
            Grain::Hour.records_predicate(&id("d"), ColumnKind::Date, &ctx, "2026-03-01 10:00:00"),
            None
        );
    }

    #[test]
    fn which_kinds_and_columns_take_which_grain() {
        use ChartKind::{Bar, Calendar, Geomap, Kpi, Scatter, Table, Text};
        for kind in [
            Bar,
            ChartKind::Line,
            ChartKind::Heatmap,
            ChartKind::Radar,
            ChartKind::Waterfall,
        ] {
            for g in ALL_GRAINS {
                assert!(
                    g.supports(kind, Some(ColumnKind::DateTime)),
                    "{kind:?} {g:?}"
                );
            }
        }
        for kind in [
            Scatter,
            Table,
            Kpi,
            Text,
            Geomap,
            ChartKind::Boxplot,
            ChartKind::Sankey,
        ] {
            for g in ALL_GRAINS {
                assert!(
                    !g.supports(kind, Some(ColumnKind::DateTime)),
                    "{kind:?} {g:?}"
                );
            }
        }
        for g in ALL_GRAINS {
            assert_eq!(
                g.supports(Calendar, Some(ColumnKind::DateTime)),
                g == Grain::Day,
                "{g:?}"
            );
        }
        // A plain date has no time of day.
        for g in [Grain::Minute, Grain::Hour, Grain::HourOfDay] {
            assert!(!g.supports(Bar, Some(ColumnKind::Date)), "{g:?}");
        }
        assert!(Grain::Month.supports(Bar, Some(ColumnKind::Date)));
        for column in [Some(ColumnKind::Text), Some(ColumnKind::Number), None] {
            assert!(!Grain::Month.supports(Bar, column), "{column:?}");
        }
    }

    #[test]
    fn a_refused_grain_says_why_in_plain_words() {
        let msg = |g: Grain, kind, col| g.validate(kind, col).unwrap_err();
        assert_eq!(
            msg(Grain::Month, ChartKind::Bar, Some(ColumnKind::Text)),
            "a grain needs a date or timestamp dimension."
        );
        assert_eq!(
            msg(Grain::Hour, ChartKind::Bar, Some(ColumnKind::Date)),
            "hour needs a timestamp column; this column has no time."
        );
        assert_eq!(
            msg(Grain::HourOfDay, ChartKind::Bar, Some(ColumnKind::Date)),
            "hour of day needs a timestamp column; this column has no time."
        );
        assert_eq!(
            msg(Grain::Month, ChartKind::Calendar, Some(ColumnKind::Date)),
            "a calendar chart can only group by day."
        );
        assert_eq!(
            msg(Grain::Month, ChartKind::Scatter, Some(ColumnKind::Date)),
            "this chart type cannot group dates."
        );
        assert!(
            Grain::Day
                .validate(ChartKind::Calendar, Some(ColumnKind::DateTime))
                .is_ok()
        );
        assert!(
            Grain::Month
                .validate(ChartKind::Bar, Some(ColumnKind::Date))
                .is_ok()
        );
    }

    #[test]
    fn the_dashboard_switch_replaces_a_truncation_and_leaves_parts_and_plain_charts_alone() {
        let date = Some(ColumnKind::Date);
        let line = ChartKind::Line;
        let r = resolve(Some("day"), line, date, Some(Grain::Month));
        assert_eq!((r.effective, r.skipped), (Some(Grain::Month), None));
        let r = resolve(Some("day_of_week"), line, date, Some(Grain::Month));
        assert_eq!(r.effective, Some(Grain::DayOfWeek), "a part is untouched");
        let r = resolve(None, line, date, Some(Grain::Month));
        assert_eq!(r, Resolution::default(), "no grain stays no grain");
        let r = resolve(Some("month"), line, date, Some(Grain::Hour));
        assert_eq!(
            (r.effective, r.skipped),
            (Some(Grain::Month), Some(Grain::Hour)),
            "hour on a Date keeps the chart's own and is reported"
        );
        let r = resolve(Some("day"), ChartKind::Calendar, date, Some(Grain::Month));
        assert_eq!(
            (r.effective, r.skipped),
            (Some(Grain::Day), Some(Grain::Month)),
            "a calendar takes day only"
        );
        let r = resolve(Some("month"), line, date, Some(Grain::Month));
        assert_eq!((r.effective, r.skipped), (Some(Grain::Month), None));
        let r = resolve(Some("month"), line, date, Some(Grain::DayOfWeek));
        assert_eq!(r.effective, Some(Grain::Month), "a part is never a switch");
    }

    #[test]
    fn a_grain_the_column_no_longer_fits_is_dropped_not_built() {
        let r = resolve(Some("month"), ChartKind::Bar, Some(ColumnKind::Text), None);
        assert_eq!(r, Resolution::default());
        let r = resolve(Some("month"), ChartKind::Bar, None, Some(Grain::Day));
        assert_eq!(r, Resolution::default());
        let r = resolve(
            Some("nonsense"),
            ChartKind::Bar,
            Some(ColumnKind::Date),
            None,
        );
        assert_eq!(r, Resolution::default());
    }

    #[test]
    fn a_grained_chart_may_ask_for_a_thousand_buckets() {
        assert_eq!(clamp_limit(ChartKind::Line, None), 20);
        assert_eq!(clamp_limit(ChartKind::Calendar, None), 366);
        assert_eq!(clamp_limit(ChartKind::Line, Some(0)), 1);
        assert_eq!(clamp_limit(ChartKind::Line, Some(1000)), 1000);
        assert_eq!(clamp_limit(ChartKind::Line, Some(5000)), 1000);
        assert!(keeps_latest("none"));
        // R1: a part ordered by bucket is never cut; a truncation is.
        for part in [
            Grain::HourOfDay,
            Grain::DayOfWeek,
            Grain::DayOfMonth,
            Grain::WeekOfYear,
            Grain::MonthOfYear,
            Grain::QuarterOfYear,
        ] {
            assert!(!cuts_to_latest(part, "none"), "{part:?}");
            assert_eq!(
                effective_limit(part, "none", 5),
                PART_RANGE_LIMIT,
                "{part:?}"
            );
            assert!(effective_limit(part, "none", 5) >= 54);
            // a value order keeps the editor's top N
            assert_eq!(effective_limit(part, "desc", 5), 5, "{part:?}");
        }
        assert!(cuts_to_latest(Grain::Month, "none"));
        assert!(!cuts_to_latest(Grain::Month, "desc"));
        assert_eq!(effective_limit(Grain::Month, "none", 5), 5);
        assert!(!keeps_latest("desc"));
        assert!(!keeps_latest("asc"));
    }

    fn rows(xs: &[Value]) -> Vec<Map<String, Value>> {
        xs.iter()
            .map(|x| {
                let mut m = Map::new();
                m.insert("x".to_owned(), x.clone());
                m.insert("v".to_owned(), json!(1));
                m
            })
            .collect()
    }

    #[test]
    fn the_surplus_oldest_bucket_is_dropped_and_the_cut_is_reported() {
        let mut r = rows(&[
            json!("2026-01-01"),
            json!("2026-02-01"),
            json!("2026-03-01"),
        ]);
        assert!(trim_to_latest(&mut r, "x", 2));
        let kept: Vec<_> = r.iter().map(|m| m["x"].clone()).collect();
        assert_eq!(kept, [json!("2026-02-01"), json!("2026-03-01")]);

        let mut r = rows(&[json!("2026-01-01"), json!("2026-02-01")]);
        assert!(
            !trim_to_latest(&mut r, "x", 2),
            "exactly the limit is not a cut"
        );
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn every_row_of_a_dropped_bucket_goes_with_it() {
        // A breakdown puts several rows on one bucket.
        let mut r = rows(&[json!(1), json!(1), json!(2), json!(2), json!(3)]);
        assert!(trim_to_latest(&mut r, "x", 2));
        let kept: Vec<_> = r.iter().map(|m| m["x"].clone()).collect();
        assert_eq!(kept, [json!(2), json!(2), json!(3)]);
    }

    #[test]
    fn the_empty_bucket_is_dropped_before_a_dated_one() {
        let mut r = rows(&[json!("2026-01-01"), json!("2026-02-01"), Value::Null]);
        assert!(trim_to_latest(&mut r, "x", 2));
        let kept: Vec<_> = r.iter().map(|m| m["x"].clone()).collect();
        assert_eq!(kept, [json!("2026-01-01"), json!("2026-02-01")]);
    }
}
