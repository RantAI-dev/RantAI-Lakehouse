//! Dashboard filter definitions: what a filter can say, whether it is
//! well-formed, and what kind of column it may be applied to.
//!
//! BI-18 part A (`docs/superpowers/plans/2026-10-07-dashboard-filters.md`).
//! A filter used to be "column is one of these values". It is now typed
//! (`op`): value lists, number and date ranges, dates relative to today, and
//! text matches. Storage did not change: boards keep their filters as a JSON
//! string and every new field is optional, so a filter saved before this
//! change (`{"column", "values"}`) still deserializes to `op = in` and
//! serializes back to the same bytes.
//!
//! Validation lives here and not in the builder because it needs no column
//! types: a malformed filter is the caller's mistake (a 400 with our own
//! message) and must never reach SQL. Whether a well-formed filter *fits* a
//! relation's column is decided later, per relation, in
//! [`crate::builder`] — see [`ColumnKind`].

use lakehouse_core::ident::Ident;
use serde::{Deserialize, Serialize};

/// Most boards' filters the API accepts in one list.
pub const MAX_FILTERS: usize = 20;
/// Most values one `in` / `not_in` filter may carry.
pub const MAX_VALUES: usize = 500;
/// Longest text a `contains` / `starts_with` / `ends_with` filter may carry,
/// in characters.
pub const MAX_TEXT_CHARS: usize = 200;
/// Largest `n` of a "last n units" filter.
pub const MAX_RELATIVE_N: u32 = 3650;

/// Years a date bound may name. `ClickHouse` `Date32` covers 1900..=2299; a
/// bound outside it would be silently clamped by the server, so it is
/// rejected here instead.
const DATE_YEARS: std::ops::RangeInclusive<u32> = 1900..=2299;

/// How a filter compares a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterOp {
    /// Column is one of `values`. The default, and the only op a filter
    /// saved before BI-18 can have.
    #[default]
    In,
    /// Column is none of `values`.
    NotIn,
    /// Column lies between `min` and `max`, either end optional.
    Between,
    /// Date column lies in a period relative to today.
    Relative,
    /// Text column contains `text`, case-insensitively.
    Contains,
    /// Text column starts with `text`, case-insensitively.
    StartsWith,
    /// Text column ends with `text`, case-insensitively.
    EndsWith,
    /// Text column does not contain `text`, case-insensitively. A NULL or
    /// empty value is kept: it does not contain the text (BI-18 round two).
    NotContains,
}

impl FilterOp {
    /// `skip_serializing_if` helper: the default op is left off the wire so
    /// a legacy filter round-trips unchanged.
    #[must_use]
    #[allow(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if hands the field by reference"
    )]
    pub fn is_in(&self) -> bool {
        *self == Self::In
    }
}

/// The unit of a [`FilterOp::Relative`] filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelativeUnit {
    /// Calendar day.
    Day,
    /// Week, starting Monday.
    Week,
    /// Calendar month.
    Month,
    /// Calendar quarter.
    Quarter,
    /// Calendar year.
    Year,
}

/// Which period a [`FilterOp::Relative`] filter selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelativeAnchor {
    /// The `n` units ending today, today included.
    Last,
    /// The current calendar unit.
    This,
    /// The calendar unit before the current one.
    Previous,
    /// The `n` units starting tomorrow, today excluded (BI-18 round two).
    Next,
}

/// A dashboard filter, applied to every tile whose data has the column.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterDef {
    /// The column to filter on.
    pub column: String,
    /// Values for `in` / `not_in`. Always serialized, as before BI-18.
    #[serde(default)]
    pub values: Vec<String>,
    /// How the column is compared. Absent means `in`.
    #[serde(default, skip_serializing_if = "FilterOp::is_in")]
    pub op: FilterOp,
    /// Lower bound of `between`: a number, or an ISO date `YYYY-MM-DD`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<String>,
    /// Upper bound of `between`, same format as `min`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<String>,
    /// Unit of `relative`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<RelativeUnit>,
    /// Count of units for `relative` with anchor `last`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>,
    /// Period selected by `relative`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<RelativeAnchor>,
    /// Needle of `contains` / `starts_with` / `ends_with`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// `between` leaves out `min` itself (a "greater than" / "after").
    /// BI-18 round two: `between` was inclusive only, and "after 3 Oct" must
    /// not include 3 Oct; skipped when false so older filters keep their bytes.
    #[serde(default, skip_serializing_if = "is_false")]
    pub min_exclusive: bool,
    /// `between` leaves out `max` itself (a "less than" / "before").
    #[serde(default, skip_serializing_if = "is_false")]
    pub max_exclusive: bool,
    /// The console may not remove this filter. Meaningful only on a board's
    /// saved default; the server keeps it and never reads it when building
    /// SQL, so public and embed views (which use the default) are unaffected.
    #[serde(default, skip_serializing_if = "is_false")]
    pub required: bool,
}

#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if hands the field by reference"
)]
fn is_false(b: &bool) -> bool {
    !*b
}

impl FilterDef {
    /// An `in` filter: the shape every filter had before BI-18, and the one
    /// an embed token's locked `params` become.
    #[must_use]
    pub fn in_values(column: impl Into<String>, values: Vec<String>) -> Self {
        Self {
            column: column.into(),
            values,
            op: FilterOp::In,
            min: None,
            max: None,
            unit: None,
            n: None,
            anchor: None,
            text: None,
            min_exclusive: false,
            max_exclusive: false,
            required: false,
        }
    }

    /// Whether the filter restricts anything. An `in` with no values, a
    /// `between` with neither bound and a text op with no text are
    /// placeholders a console may hold while the user is still choosing;
    /// they never reach SQL and are not reported as skipped.
    #[must_use]
    pub fn is_active(&self) -> bool {
        match self.op {
            FilterOp::In | FilterOp::NotIn => !self.values.is_empty(),
            FilterOp::Between => self.min.is_some() || self.max.is_some(),
            FilterOp::Relative => self.unit.is_some() && self.anchor.is_some(),
            FilterOp::Contains
            | FilterOp::StartsWith
            | FilterOp::EndsWith
            | FilterOp::NotContains => self.text.as_deref().is_some_and(|t| !t.is_empty()),
        }
    }

    /// Check the filter is well-formed: the fields its op needs are present
    /// and parse. It does not know the column's type; that is checked per
    /// relation when the predicate is built.
    ///
    /// # Errors
    ///
    /// A message for the caller, naming what is wrong. It never echoes the
    /// offending value.
    pub fn validate(&self) -> Result<(), String> {
        if Ident::new(self.column.clone()).is_err() {
            return Err("the column name is not a valid identifier".to_owned());
        }
        if self.values.len() > MAX_VALUES {
            return Err(format!("at most {MAX_VALUES} values per filter"));
        }
        match self.op {
            FilterOp::In | FilterOp::NotIn => Ok(()),
            FilterOp::Between => self.validate_between(),
            FilterOp::Relative => self.validate_relative(),
            FilterOp::Contains
            | FilterOp::StartsWith
            | FilterOp::EndsWith
            | FilterOp::NotContains => {
                let Some(text) = self.text.as_deref() else {
                    return Err("a text filter needs `text`".to_owned());
                };
                if text.is_empty() {
                    return Err("a text filter needs non-empty `text`".to_owned());
                }
                if text.chars().count() > MAX_TEXT_CHARS {
                    return Err(format!("`text` is at most {MAX_TEXT_CHARS} characters"));
                }
                Ok(())
            }
        }
    }

    fn validate_between(&self) -> Result<(), String> {
        if self.min.is_none() && self.max.is_none() {
            return Err("a range filter needs `min`, `max`, or both".to_owned());
        }
        // An exclusive end with no end to exclude is a malformed pick.
        if (self.min_exclusive && self.min.is_none()) || (self.max_exclusive && self.max.is_none())
        {
            return Err("an exclusive end needs its bound".to_owned());
        }
        let mut seen_number = false;
        let mut seen_date = false;
        for (name, bound) in [("min", &self.min), ("max", &self.max)] {
            let Some(raw) = bound.as_deref() else {
                continue;
            };
            if parse_number(raw).is_some() {
                seen_number = true;
            } else if parse_iso_date(raw).is_some() {
                seen_date = true;
            } else {
                return Err(format!(
                    "`{name}` must be a number or a date as YYYY-MM-DD (1900 to 2299)"
                ));
            }
        }
        if seen_number && seen_date {
            return Err("`min` and `max` must both be numbers or both be dates".to_owned());
        }
        Ok(())
    }

    fn validate_relative(&self) -> Result<(), String> {
        let (Some(_), Some(anchor)) = (self.unit, self.anchor) else {
            return Err("a relative filter needs `unit` and `anchor`".to_owned());
        };
        match (anchor, self.n) {
            (RelativeAnchor::Last, None) => Err("`last` needs `n`".to_owned()),
            (RelativeAnchor::Next, None) => Err("`next` needs `n`".to_owned()),
            (_, Some(n)) if !(1..=MAX_RELATIVE_N).contains(&n) => {
                Err(format!("`n` must be between 1 and {MAX_RELATIVE_N}"))
            }
            _ => Ok(()),
        }
    }
}

/// Validate the filters of one board or request.
///
/// # Errors
///
/// A message for the caller: more than [`MAX_FILTERS`] filters, or the first
/// malformed one (by 1-based position).
pub fn validate_filters(filters: &[FilterDef]) -> Result<(), String> {
    if filters.len() > MAX_FILTERS {
        return Err(format!("at most {MAX_FILTERS} filters"));
    }
    for (i, f) in filters.iter().enumerate() {
        f.validate()
            .map_err(|msg| format!("filter {}: {msg}", i + 1))?;
    }
    Ok(())
}

/// The filters that restrict something, in order. An `in` with no values
/// (the pre-BI-18 bar stored a column with an empty selection that way), a
/// `between` with neither bound and a text op with no text filter nothing, so
/// they are not stored (BI-18 review round 1). A `required` flag on such a
/// placeholder goes with it: a filter that restricts nothing cannot be
/// required.
#[must_use]
pub fn without_inert(filters: &[FilterDef]) -> Vec<FilterDef> {
    filters.iter().filter(|f| f.is_active()).cloned().collect()
}

/// A finite number, as text. `NaN` and infinities are refused so a bound can
/// always be rendered as a plain numeric literal.
pub(crate) fn parse_number(raw: &str) -> Option<f64> {
    let v: f64 = raw.trim().parse().ok()?;
    v.is_finite().then_some(v)
}

/// A calendar date written exactly `YYYY-MM-DD`, as `(year, month, day)`.
///
/// Parsed by hand rather than passed to `ClickHouse` because `toDate32`
/// silently rolls `2024-02-30` over to March: the check must happen before
/// the value is rendered.
pub(crate) fn parse_iso_date(raw: &str) -> Option<(u32, u32, u32)> {
    let b = raw.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let digits = |s: &str| -> Option<u32> {
        s.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| s.parse().ok())
            .flatten()
    };
    let (year, month, day) = (
        digits(raw.get(0..4)?)?,
        digits(raw.get(5..7)?)?,
        digits(raw.get(8..10)?)?,
    );
    if !DATE_YEARS.contains(&year) || !(1..=12).contains(&month) {
        return None;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let last_day = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    (1..=last_day).contains(&day).then_some((year, month, day))
}

/// What a column holds, as far as filtering cares. Derived from the
/// `ClickHouse` type string, never from the column's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ColumnKind {
    /// `Int*`, `UInt*`, `Float*`, `Decimal*`.
    Number,
    /// `Date`, `Date32`.
    Date,
    /// `DateTime`, `DateTime64`, with or without a time zone.
    DateTime,
    /// Everything else (`String`, `Enum`, `UUID`, `Bool`, ...), compared as
    /// its `toString`.
    Text,
}

impl ColumnKind {
    /// The kind of a `ClickHouse` type string such as
    /// `LowCardinality(Nullable(String))`.
    #[must_use]
    pub fn from_clickhouse_type(ty: &str) -> Self {
        let mut ty = ty.trim();
        // `Nullable` and `LowCardinality` wrap the value type and can nest
        // either way round.
        while let Some(inner) = ["Nullable(", "LowCardinality("]
            .iter()
            .find_map(|w| ty.strip_prefix(w))
            .and_then(|rest| rest.strip_suffix(')'))
        {
            ty = inner.trim();
        }
        let all_digits = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
        // `Interval*` also starts with `Int`, hence the digit check.
        let numbered = |prefix: &str| ty.strip_prefix(prefix).is_some_and(all_digits);
        if numbered("Int") || numbered("UInt") || numbered("Float") || ty == "BFloat16" {
            return Self::Number;
        }
        if ty == "Decimal"
            || ty.strip_prefix("Decimal").is_some_and(|rest| {
                rest.starts_with('(') || all_digits(rest.split('(').next().unwrap_or(""))
            })
        {
            return Self::Number;
        }
        if ty == "Date" || ty == "Date32" {
            return Self::Date;
        }
        if ty == "DateTime" || ty.starts_with("DateTime(") || ty.starts_with("DateTime64") {
            return Self::DateTime;
        }
        Self::Text
    }

    /// Whether a date range or relative date applies.
    #[must_use]
    pub fn is_temporal(self) -> bool {
        matches!(self, Self::Date | Self::DateTime)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn parse(json: &str) -> FilterDef {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn a_filter_saved_before_typed_filters_is_an_in_filter_and_round_trips_unchanged() {
        let legacy = r#"{"column":"kawasan","values":["Asia","Eropa"]}"#;
        let f = parse(legacy);
        assert_eq!(f.op, FilterOp::In);
        assert_eq!(
            f,
            FilterDef::in_values("kawasan", vec!["Asia".into(), "Eropa".into()])
        );
        assert_eq!(serde_json::to_string(&f).unwrap(), legacy);
    }

    #[test]
    fn round_two_fields_round_trip_and_stay_off_the_wire_when_false() {
        let json = r#"{"column":"d","values":[],"op":"between","min":"2026-10-03","minExclusive":true,"required":true}"#;
        let f = parse(json);
        assert!(f.min_exclusive && !f.max_exclusive && f.required);
        assert_eq!(serde_json::to_string(&f).unwrap(), json);
        // A round-one typed filter keeps its exact bytes.
        let one =
            r#"{"column":"d","values":[],"op":"between","min":"2026-10-03","max":"2026-10-09"}"#;
        assert_eq!(serde_json::to_string(&parse(one)).unwrap(), one);
        let next =
            r#"{"column":"d","values":[],"op":"relative","unit":"day","n":7,"anchor":"next"}"#;
        assert_eq!(serde_json::to_string(&parse(next)).unwrap(), next);
    }

    #[test]
    fn an_unknown_op_is_a_parse_error_not_a_silent_in() {
        let bad = r#"{"column":"c","values":[],"op":"regex"}"#;
        assert!(serde_json::from_str::<FilterDef>(bad).is_err());
    }

    #[test]
    fn typed_fields_use_camel_case_names_and_are_omitted_when_absent() {
        let f = parse(r#"{"column":"d","op":"relative","anchor":"last","n":30,"unit":"day"}"#);
        let out = serde_json::to_value(&f).unwrap();
        assert_eq!(out["op"], "relative");
        assert_eq!(out["unit"], "day");
        assert_eq!(out["n"], 30);
        assert!(out.get("min").is_none());
        assert_eq!(out["values"], serde_json::json!([]));
    }

    #[test]
    fn every_op_validates_when_complete() {
        for json in [
            r#"{"column":"c","values":["a"]}"#,
            r#"{"column":"c","op":"not_in","values":["a"]}"#,
            r#"{"column":"c","op":"between","min":"1","max":"2.5"}"#,
            r#"{"column":"c","op":"between","min":"2024-01-01"}"#,
            r#"{"column":"c","op":"between","max":"-3"}"#,
            r#"{"column":"c","op":"relative","anchor":"last","n":7,"unit":"day"}"#,
            r#"{"column":"c","op":"relative","anchor":"this","unit":"month"}"#,
            r#"{"column":"c","op":"relative","anchor":"previous","unit":"year"}"#,
            r#"{"column":"c","op":"contains","text":"bal"}"#,
            r#"{"column":"c","op":"starts_with","text":"b"}"#,
            r#"{"column":"c","op":"ends_with","text":"i"}"#,
            r#"{"column":"c","op":"not_contains","text":"i"}"#,
            r#"{"column":"c","op":"relative","anchor":"next","n":7,"unit":"day"}"#,
            r#"{"column":"c","op":"between","min":"1","minExclusive":true}"#,
            r#"{"column":"c","op":"between","max":"2024-01-01","maxExclusive":true,"required":true}"#,
        ] {
            assert_eq!(parse(json).validate(), Ok(()), "{json}");
        }
    }

    #[test]
    fn malformed_filters_are_rejected() {
        for json in [
            r#"{"column":"bad col","values":["a"]}"#,
            r#"{"column":"c","op":"between"}"#,
            r#"{"column":"c","op":"between","min":"abc"}"#,
            r#"{"column":"c","op":"between","min":"NaN"}"#,
            r#"{"column":"c","op":"between","min":"inf"}"#,
            r#"{"column":"c","op":"between","min":"2024-02-30"}"#,
            r#"{"column":"c","op":"between","min":"2024-1-1"}"#,
            r#"{"column":"c","op":"between","min":"1899-12-31"}"#,
            r#"{"column":"c","op":"between","min":"1","max":"2024-01-01"}"#,
            r#"{"column":"c","op":"relative","unit":"day"}"#,
            r#"{"column":"c","op":"relative","anchor":"last","unit":"day"}"#,
            r#"{"column":"c","op":"relative","anchor":"last","n":0,"unit":"day"}"#,
            r#"{"column":"c","op":"relative","anchor":"last","n":3651,"unit":"day"}"#,
            r#"{"column":"c","op":"contains"}"#,
            r#"{"column":"c","op":"contains","text":""}"#,
            r#"{"column":"c","op":"not_contains"}"#,
            r#"{"column":"c","op":"relative","anchor":"next","unit":"day"}"#,
            r#"{"column":"c","op":"relative","anchor":"next","n":0,"unit":"day"}"#,
            r#"{"column":"c","op":"between","max":"3","minExclusive":true}"#,
            r#"{"column":"c","op":"between","min":"3","maxExclusive":true,"max":"x"}"#,
        ] {
            assert!(parse(json).validate().is_err(), "{json}");
        }
    }

    #[test]
    fn text_is_capped_at_200_characters_and_values_at_500() {
        let long = format!(
            r#"{{"column":"c","op":"contains","text":"{}"}}"#,
            "é".repeat(201)
        );
        assert!(parse(&long).validate().is_err());
        let ok = format!(
            r#"{{"column":"c","op":"contains","text":"{}"}}"#,
            "é".repeat(200)
        );
        assert!(parse(&ok).validate().is_ok());
        let many = FilterDef::in_values("c", (0..=MAX_VALUES).map(|i| i.to_string()).collect());
        assert!(many.validate().is_err());
    }

    #[test]
    fn a_board_may_carry_twenty_filters_and_no_more() {
        let one = FilterDef::in_values("c", vec!["a".into()]);
        assert!(validate_filters(&vec![one.clone(); MAX_FILTERS]).is_ok());
        assert!(validate_filters(&vec![one; MAX_FILTERS + 1]).is_err());
        let bad = FilterDef::in_values("bad col", vec![]);
        let err = validate_filters(&[bad]).unwrap_err();
        assert!(err.starts_with("filter 1:"), "{err}");
    }

    #[test]
    fn an_empty_placeholder_filter_is_valid_but_not_active() {
        let f = FilterDef::in_values("c", vec![]);
        assert!(f.validate().is_ok());
        assert!(!f.is_active());
        assert!(FilterDef::in_values("c", vec!["a".into()]).is_active());
    }

    #[test]
    fn inert_filters_are_dropped_before_storing_and_active_ones_kept_in_order() {
        let mut required_empty = FilterDef::in_values("r", vec![]);
        required_empty.required = true;
        let keep = parse(r#"{"column":"k","values":["a"]}"#);
        let range = parse(r#"{"column":"p","op":"between","min":"1"}"#);
        let list = vec![
            FilterDef::in_values("visit_date", vec![]),
            keep.clone(),
            parse(r#"{"column":"visitors","values":[],"op":"not_in"}"#),
            required_empty,
            range.clone(),
        ];
        assert_eq!(without_inert(&list), vec![keep, range]);
        assert!(without_inert(&[]).is_empty());
    }

    #[test]
    fn iso_dates_are_checked_against_the_calendar() {
        assert_eq!(parse_iso_date("2024-02-29"), Some((2024, 2, 29)));
        assert_eq!(parse_iso_date("2023-02-29"), None);
        assert_eq!(
            parse_iso_date("1900-02-29"),
            None,
            "1900 is not a leap year"
        );
        assert_eq!(parse_iso_date("2000-02-29"), Some((2000, 2, 29)));
        assert_eq!(parse_iso_date("2024-13-01"), None);
        assert_eq!(parse_iso_date("2024-04-31"), None);
        assert_eq!(parse_iso_date("2024-01-01'; DROP"), None);
        assert_eq!(parse_iso_date("+024-01-01"), None);
    }

    #[test]
    fn column_kinds_come_from_the_clickhouse_type() {
        use ColumnKind::{Date, DateTime, Number, Text};
        for (ty, want) in [
            ("UInt16", Number),
            ("Int64", Number),
            ("Float64", Number),
            ("Decimal(10, 2)", Number),
            ("Decimal64(4)", Number),
            ("Nullable(Int32)", Number),
            ("LowCardinality(Nullable(String))", Text),
            ("Nullable(LowCardinality(UInt8))", Number),
            ("Date", Date),
            ("Date32", Date),
            ("Nullable(Date)", Date),
            ("DateTime", DateTime),
            ("DateTime('Asia/Jakarta')", DateTime),
            ("DateTime64(3)", DateTime),
            ("DateTime64(3, 'UTC')", DateTime),
            ("String", Text),
            ("UUID", Text),
            ("Bool", Text),
            ("Enum8('a' = 1)", Text),
            ("IntervalDay", Text),
            ("Array(Int32)", Text),
            ("", Text),
        ] {
            assert_eq!(ColumnKind::from_clickhouse_type(ty), want, "{ty}");
        }
    }
}
