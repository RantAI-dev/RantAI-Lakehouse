//! The fields of the progress, histogram, bullet, image and web cards
//! (`BI-16` part B, decisions D1 to D5 of
//! `docs/superpowers/plans/2026-10-15-bi-16b-simple-types.md`).
//!
//! Flattened into [`crate::store::ChartInput`] like [`crate::tables::TableFields`]
//! and [`crate::style::StyleFields`], so the wire shape stays flat and a chart
//! saved before they existed reads back with every one absent. Every field is
//! refused on a kind that does not take it, so it is never stored and ignored.
//!
//! # Why the checks are what they are
//!
//! - **`goal`**: a progress bar and a bullet graph compare a value with a goal
//!   that is either a fixed number or a second value of the same source. A
//!   second value is the chart's second measure (`measures: [value, goal]`) and
//!   `goal: {"measure": "<its column>"}` says so; the statement then returns
//!   both numbers in one row, so a filter narrows them together.
//! - **`histogram`**: the width of the ranges, or how many, never both. A
//!   number of ranges is honoured (`BinMode::Ranges`: that many equal ranges
//!   over the shown minimum to maximum, the width rounded up to two
//!   significant digits), and neither means automatic ranges (`BinMode::Nearest`:
//!   the nice width whose ranges are closest to [`AUTO_HISTOGRAM_BINS`]); both
//!   are `BI-18` part C review fix (SHOULD-FIX) R21, because "at most N" gave 5
//!   ranges for 10 and 12 for "about 20". `BI-10`'s bins aim at about ten and do
//!   not change.
//! - **`url`** (image and web cards): the card loads a page or a picture into
//!   every viewer's browser, so the address is checked here, at save, for both
//!   the console and the assistant: `https` only, a plain host, no user-info
//!   (`https://allowed.example@evil.example/` hides its host there), and its
//!   origin must be in the administrator's list of content sites. The message
//!   is one fixed sentence and never repeats the address (`SEC-11`). The
//!   console checks the list again when it draws the card, so a site removed
//!   later stops loading; no `Content-Security-Policy` enforces the list in
//!   the browser yet (a separate backlog item).

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::bin::{BinMode, MAX_WIDTH, MIN_WIDTH};
use crate::embed_access::https_origin_of;
use crate::specs::ChartKind;
use crate::store::ChartInput;

/// The ranges a histogram aims at when it is not told a width or a count.
pub const AUTO_HISTOGRAM_BINS: u32 = 20;

/// The most ranges a histogram may ask for.
pub const MAX_HISTOGRAM_COUNT: u32 = 100;

/// The most bands a bullet graph draws.
pub const MAX_BANDS: usize = 3;

/// The longest description of an image, in characters.
pub const MAX_ALT_CHARS: usize = 200;

/// The longest address of an image or web card, in characters.
pub const MAX_URL_CHARS: usize = 2000;

/// What a person is told when an image or web card's address is not on the
/// administrator's list. One sentence for the console and the assistant, and
/// it never repeats the address.
pub const CONTENT_SITE_REFUSED: &str = "Images and web pages may only come from the sites an \
administrator allows (Admin > Settings).";

/// What a progress bar or a bullet graph is measured against.
#[derive(Debug, Clone, PartialEq)]
pub enum Goal {
    /// A fixed number.
    Fixed(f64),
    /// The chart's second measure, named by its column.
    Value(String),
}

/// A number as `JSON.stringify` writes it: a whole value is an integer.
fn js_number<S: Serializer>(value: f64, serializer: S) -> Result<S::Ok, S::Error> {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "checked: a whole number below 1e15 is exact in an i64"
    )]
    if value.is_finite() && value.fract() == 0.0 && value.abs() < 1e15 {
        serializer.serialize_i64(value as i64)
    } else {
        serializer.serialize_f64(value)
    }
}

impl Serialize for Goal {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        match self {
            Self::Fixed(v) => js_number(*v, serializer),
            Self::Value(column) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("measure", column)?;
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for Goal {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        const SHAPE: &str = "goal must be a number or {\"measure\": \"<column>\"}.";
        match serde_json::Value::deserialize(deserializer)? {
            serde_json::Value::Number(n) => n
                .as_f64()
                .filter(|v| v.is_finite())
                .map(Self::Fixed)
                .ok_or_else(|| D::Error::custom(SHAPE)),
            // The assistant's model sometimes sends a number as text.
            serde_json::Value::String(s) => s
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .map(Self::Fixed)
                .ok_or_else(|| D::Error::custom(SHAPE)),
            serde_json::Value::Object(map) if map.len() == 1 => match map.get("measure") {
                Some(serde_json::Value::String(column)) => Ok(Self::Value(column.clone())),
                _ => Err(D::Error::custom(SHAPE)),
            },
            _ => Err(D::Error::custom(SHAPE)),
        }
    }
}

/// The ranges of a histogram: a width, or how many at most, not both.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistogramBins {
    /// The width of each range.
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        deserialize_with = "loose_number",
        serialize_with = "crate::store::serialize_js_number"
    )]
    pub width: Option<f64>,
    /// At most this many ranges, 1 to [`MAX_HISTOGRAM_COUNT`].
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        deserialize_with = "loose_number_u32"
    )]
    pub count: Option<u32>,
}

/// A number, or a number written as text, or nothing.
fn loose_number<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<f64>, D::Error> {
    match Option::<serde_json::Value>::deserialize(deserializer)? {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Number(n)) => Ok(n.as_f64().filter(|v| v.is_finite())),
        Some(serde_json::Value::String(s)) => s
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .map(Some)
            .ok_or_else(|| D::Error::custom("the histogram's width and count must be numbers.")),
        Some(_) => Err(D::Error::custom(
            "the histogram's width and count must be numbers.",
        )),
    }
}

/// [`loose_number`] for a whole count. A fraction or a negative number is kept
/// out of range (`u32::MAX`) so [`check_fields`] refuses it with its sentence
/// instead of truncating it into another count.
fn loose_number_u32<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u32>, D::Error> {
    Ok(loose_number(deserializer)?.map(|v| {
        if v.fract() == 0.0 && (0.0..=f64::from(u32::MAX - 1)).contains(&v) {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "checked just above: a whole number between 0 and u32::MAX - 1"
            )]
            {
                v as u32
            }
        } else {
            u32::MAX
        }
    }))
}

impl HistogramBins {
    /// The mode the ranges are cut with: a fixed width, `count` ranges, or the
    /// nice width nearest [`AUTO_HISTOGRAM_BINS`] ranges.
    ///
    /// # Errors
    ///
    /// The sentence to show the person who chose the ranges.
    pub fn mode(&self) -> Result<BinMode, String> {
        match (self.width, self.count) {
            (Some(_), Some(_)) => {
                Err("give a histogram a width or a number of ranges, not both.".to_owned())
            }
            (Some(w), None) if (MIN_WIDTH..=MAX_WIDTH).contains(&w) => Ok(BinMode::Width(w)),
            (Some(_), None) => Err(format!(
                "a histogram's width must be between {MIN_WIDTH} and {MAX_WIDTH}."
            )),
            (None, Some(c)) if (1..=MAX_HISTOGRAM_COUNT).contains(&c) => Ok(BinMode::Ranges(c)),
            (None, Some(_)) => Err(format!(
                "a histogram has 1 to {MAX_HISTOGRAM_COUNT} ranges."
            )),
            (None, None) => Ok(BinMode::Nearest(AUTO_HISTOGRAM_BINS)),
        }
    }
}

/// The fields `BI-16` part B adds to a chart definition.
///
/// [`ChartInput`] holds them boxed: a definition is carried through the
/// assistant's tool futures, and a hundred more bytes in every copy pushed the
/// agent loop's future over clippy's `large_futures` limit.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TypeFields {
    /// `progress`, `bullet`: what the value is measured against.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub goal: Option<Goal>,
    /// `bullet`: 1 to 3 ascending upper limits of the bands.
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        serialize_with = "serialize_bands"
    )]
    pub bands: Option<Vec<f64>>,
    /// `histogram`: the ranges; absent is automatic.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub histogram: Option<HistogramBins>,
    /// `image`, `web`: the address.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub url: Option<String>,
    /// `image`: the description for screen readers.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub alt: Option<String>,
    /// `gantt`: the column a task starts at (a date or date-time), required
    /// (`BI-16` part C1, plan D1).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub start: Option<String>,
    /// `gantt`: the column a task ends at (a date or date-time), required.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub end: Option<String>,
    /// `gantt`: an optional number column, 0 to 100 or 0 to 1, drawn as shading.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub progress: Option<String>,
}

#[allow(
    clippy::ref_option,
    reason = "serde's `serialize_with` contract requires `&Option<Vec<f64>>`"
)]
fn serialize_bands<S: Serializer>(
    value: &Option<Vec<f64>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    struct Num(f64);
    impl Serialize for Num {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            js_number(self.0, serializer)
        }
    }
    match value {
        None => serializer.serialize_none(),
        Some(list) => {
            let mut seq = serializer.serialize_seq(Some(list.len()))?;
            for v in list {
                seq.serialize_element(&Num(*v))?;
            }
            seq.end()
        }
    }
}

impl TypeFields {
    /// Whether any field is present.
    #[must_use]
    pub fn is_used(&self) -> bool {
        self.goal.is_some()
            || self.bands.is_some()
            || self.histogram.is_some()
            || self.url.is_some()
            || self.alt.is_some()
            || self.start.is_some()
            || self.end.is_some()
            || self.progress.is_some()
    }

    /// The second measure's column when the goal is a value.
    #[must_use]
    pub fn goal_measure(&self) -> Option<&str> {
        match &self.goal {
            Some(Goal::Value(column)) => Some(column.as_str()),
            _ => None,
        }
    }
}

fn kind_word(kind: ChartKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// Whether a bullet graph has no category: one bullet, the single-number path.
#[must_use]
pub fn is_single_bullet(input: &ChartInput) -> bool {
    input.kind == ChartKind::Bullet && input.dimension.is_empty()
}

/// Whether the chart is read as one row of numbers (`v`, and `g` for a goal
/// that is a value): a progress bar, or a bullet graph without a category.
#[must_use]
pub fn is_single_row(input: &ChartInput) -> bool {
    is_single_row_kind(input.kind, input)
}

/// [`is_single_row`] for a stored chart whose kind is in its spec.
#[must_use]
pub fn is_single_row_kind(kind: ChartKind, def: &ChartInput) -> bool {
    kind == ChartKind::Progress || (kind == ChartKind::Bullet && def.dimension.is_empty())
}

/// Check which of the fields the kind takes, and the shape of each. Needs
/// neither a relation nor the list of content sites.
///
/// # Errors
///
/// The sentence to show the person who saved the chart.
pub fn check_fields(input: &ChartInput) -> Result<(), String> {
    let kind = input.kind;
    let f = &input.types;
    let only = |present: bool, field: &str, kinds: &str| {
        if present {
            Err(format!("{field} is only for {kinds}."))
        } else {
            Ok(())
        }
    };
    only(
        f.goal.is_some() && !matches!(kind, ChartKind::Progress | ChartKind::Bullet),
        "goal",
        "a progress bar and a bullet graph",
    )?;
    only(
        f.bands.is_some() && kind != ChartKind::Bullet,
        "bands",
        "a bullet graph",
    )?;
    only(
        f.histogram.is_some() && kind != ChartKind::Histogram,
        "histogram",
        "a histogram",
    )?;
    only(
        f.url.is_some() && !matches!(kind, ChartKind::Image | ChartKind::Web),
        "url",
        "an image or web card",
    )?;
    only(
        f.alt.is_some() && kind != ChartKind::Image,
        "alt",
        "an image",
    )?;
    only(
        (f.start.is_some() || f.end.is_some() || f.progress.is_some()) && kind != ChartKind::Gantt,
        "start, end and progress",
        "a Gantt chart",
    )?;
    match kind {
        ChartKind::Gantt => check_gantt(input),
        ChartKind::Progress | ChartKind::Bullet => check_goal_and_bands(input),
        ChartKind::Histogram => check_histogram(input),
        ChartKind::Image | ChartKind::Web => check_card(input),
        _ => Ok(()),
    }
}

fn check_goal_and_bands(input: &ChartInput) -> Result<(), String> {
    let word = kind_word(input.kind);
    let f = &input.types;
    let measures = input.measures.len();
    match &f.goal {
        None if input.kind == ChartKind::Progress => {
            return Err("a progress bar needs a goal: a number, or a second value.".to_owned());
        }
        None if measures != 1 => {
            return Err(format!("a {word} without a goal takes exactly one value."));
        }
        None | Some(Goal::Fixed(_)) if measures != 1 => {
            return Err(format!(
                "a {word} with a fixed goal takes exactly one value."
            ));
        }
        Some(Goal::Value(column)) => {
            let second = input.measures.entries().nth(1).map(|(c, _)| c);
            if measures != 2 || second != Some(column.as_str()) {
                return Err(format!(
                    "the goal of a {word} measured against a value must be the second of exactly two measures."
                ));
            }
        }
        _ => {}
    }
    if input.breakdown.is_some() {
        return Err(format!("a {word} has no breakdown."));
    }
    if input.kind == ChartKind::Progress && !input.dimension.is_empty() {
        return Err("a progress bar has no category.".to_owned());
    }
    if let Some(bands) = &f.bands {
        if bands.is_empty() || bands.len() > MAX_BANDS {
            return Err(format!("a bullet graph has 1 to {MAX_BANDS} bands."));
        }
        if bands.iter().any(|b| !b.is_finite()) {
            return Err("the limits of the bands must be numbers.".to_owned());
        }
        if bands.windows(2).any(|w| w[0] >= w[1]) {
            return Err(
                "the limits of the bands must go up, each above the one before.".to_owned(),
            );
        }
    }
    Ok(())
}

/// A column name with its blanks trimmed; blank is none.
fn named(v: Option<&str>) -> Option<&str> {
    v.map(str::trim).filter(|c| !c.is_empty())
}

/// A Gantt chart (`BI-16` part C1, plan D1): its task column is the
/// dimension, its group the breakdown, and `start`, `end` (and `progress`)
/// name columns. It has no measure and no sort of its own; the statement
/// orders by start and keeps the first 1,001 rows. That the columns exist and
/// are dates and a number needs the relation ([`crate::store`]).
fn check_gantt(input: &ChartInput) -> Result<(), String> {
    let f = &input.types;
    if input.dimension.is_empty() {
        return Err("a Gantt chart needs the task column, as its dimension.".to_owned());
    }
    let (Some(start), Some(end)) = (named(f.start.as_deref()), named(f.end.as_deref())) else {
        return Err("a Gantt chart needs a start column and an end column.".to_owned());
    };
    if start == end {
        return Err("a Gantt chart's start and end must be two different columns.".to_owned());
    }
    if f.progress.is_some() && named(f.progress.as_deref()).is_none() {
        return Err(
            "the progress column of a Gantt chart is empty: choose one or remove it.".to_owned(),
        );
    }
    if !input.measures.is_empty() {
        return Err(
            "a Gantt chart takes no measure: it draws the start and end columns.".to_owned(),
        );
    }
    if input.aggregate.is_some() || input.limit.is_some() || input.order.is_some() {
        return Err(
            "a Gantt chart takes no aggregate, limit or order; it shows the first 1,000 tasks by start."
                .to_owned(),
        );
    }
    if input.grain.is_some() {
        return Err("a Gantt chart has no time grain: its dates are the bars' ends.".to_owned());
    }
    Ok(())
}

fn check_histogram(input: &ChartInput) -> Result<(), String> {
    if input.dimension.is_empty() {
        return Err("a histogram needs the number column to count, as its dimension.".to_owned());
    }
    if !input.measures.is_empty() || input.breakdown.is_some() {
        return Err(
            "a histogram counts the rows of one number column; it takes no measure or breakdown."
                .to_owned(),
        );
    }
    if input.aggregate.as_deref().is_some_and(|a| a != "count") {
        return Err("a histogram counts rows; it takes no other aggregate.".to_owned());
    }
    if input.limit.is_some() || input.order.is_some() {
        return Err(
            "a histogram takes no limit or order; its ranges are always in order.".to_owned(),
        );
    }
    if let Some(bins) = &input.types.histogram {
        bins.mode()?;
    }
    Ok(())
}

fn check_card(input: &ChartInput) -> Result<(), String> {
    let word = kind_word(input.kind);
    let f = &input.types;
    let url = f.url.as_deref().map(str::trim).unwrap_or_default();
    if url.is_empty() {
        return Err(format!("a {word} card needs an address."));
    }
    if url.chars().count() > MAX_URL_CHARS
        || url.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        // The sentence of the list check, so the address is not echoed or
        // diagnosed to someone probing what the validator accepts.
        return Err(CONTENT_SITE_REFUSED.to_owned());
    }
    if input.kind == ChartKind::Image {
        let alt = f.alt.as_deref().map(str::trim).unwrap_or_default();
        if alt.is_empty() {
            return Err("an image needs a short description for screen readers.".to_owned());
        }
        if alt.chars().count() > MAX_ALT_CHARS || alt.chars().any(char::is_control) {
            return Err(format!(
                "the description of an image is at most {MAX_ALT_CHARS} characters, on one line."
            ));
        }
    }
    if !input.dimension.is_empty() || !input.measures.is_empty() {
        return Err(format!("a {word} card has no data columns."));
    }
    Ok(())
}

/// Check an image or web card's address against the administrator's list of
/// content sites: `https`, a valid host, no user-info, and its origin listed.
/// Does nothing for the other kinds.
///
/// # Errors
///
/// [`CONTENT_SITE_REFUSED`], a fixed sentence that never repeats the address.
pub fn check_content_url(input: &ChartInput, sites: &[String]) -> Result<(), String> {
    if !matches!(input.kind, ChartKind::Image | ChartKind::Web) {
        return Ok(());
    }
    let url = input.types.url.as_deref().unwrap_or_default().trim();
    match https_origin_of(url) {
        Some(origin) if sites.contains(&origin) => Ok(()),
        _ => Err(CONTENT_SITE_REFUSED.to_owned()),
    }
}

/// The fields an image or web card keeps, normalised (the address and the
/// description trimmed); empty for every other kind.
#[must_use]
pub fn stored_card_fields(input: &ChartInput) -> TypeFields {
    let trim = |s: &Option<String>| s.as_deref().map(|v| v.trim().to_owned());
    match input.kind {
        ChartKind::Image => TypeFields {
            url: trim(&input.types.url),
            alt: trim(&input.types.alt),
            ..TypeFields::default()
        },
        ChartKind::Web => TypeFields {
            url: trim(&input.types.url),
            ..TypeFields::default()
        },
        _ => TypeFields::default(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[allow(
        clippy::needless_pass_by_value,
        reason = "every caller passes a `json!` literal"
    )]
    fn input(json: serde_json::Value) -> ChartInput {
        let mut base = serde_json::json!({"title": "t", "mart": "m"});
        base.as_object_mut()
            .unwrap()
            .extend(json.as_object().unwrap().clone());
        serde_json::from_value(base).unwrap()
    }

    fn refused(json: serde_json::Value) -> String {
        check_fields(&input(json)).unwrap_err()
    }

    #[test]
    fn a_goal_is_a_number_or_a_second_measure_and_round_trips_as_written() {
        let fixed = input(serde_json::json!({
            "kind": "progress", "measures": ["visitors"], "goal": 1_000_000
        }));
        assert_eq!(fixed.types.goal, Some(Goal::Fixed(1_000_000.0)));
        assert_eq!(serde_json::to_value(&fixed).unwrap()["goal"], 1_000_000);
        let value = input(serde_json::json!({
            "kind": "bullet", "dimension": "p", "measures": ["a", "b"],
            "goal": {"measure": "b"}, "bands": [50, 80.5]
        }));
        assert_eq!(value.types.goal, Some(Goal::Value("b".to_owned())));
        let out = serde_json::to_value(&value).unwrap();
        assert_eq!(out["goal"], serde_json::json!({"measure": "b"}));
        assert_eq!(out["bands"], serde_json::json!([50, 80.5]));
        assert!(check_fields(&fixed).is_ok());
        assert!(check_fields(&value).is_ok());
        // A number sent as text, as the assistant's model may.
        let text = input(serde_json::json!({
            "kind": "progress", "measures": ["v"], "goal": "250"
        }));
        assert_eq!(text.types.goal, Some(Goal::Fixed(250.0)));
    }

    #[test]
    fn a_field_on_the_wrong_kind_is_refused_with_the_kinds_that_take_it() {
        assert_eq!(
            refused(serde_json::json!({
                "kind": "bar", "dimension": "p", "measures": ["v"], "goal": 5
            })),
            "goal is only for a progress bar and a bullet graph."
        );
        assert!(
            refused(serde_json::json!({"kind": "kpi", "measures": ["v"], "bands": [1]}))
                .contains("only for a bullet graph")
        );
        assert!(
            refused(serde_json::json!({
                "kind": "bar", "dimension": "p", "measures": ["v"], "histogram": {"count": 5}
            }))
            .contains("only for a histogram")
        );
        assert!(
            refused(serde_json::json!({"kind": "text", "text": "x", "url": "https://a.example/"}))
                .contains("only for an image or web card")
        );
        assert!(
            refused(serde_json::json!({
                "kind": "web", "url": "https://a.example/", "alt": "x"
            }))
            .contains("alt is only for an image")
        );
    }

    #[test]
    fn a_progress_bar_needs_a_goal_and_a_goal_that_is_a_value_must_be_the_second_measure() {
        assert!(
            refused(serde_json::json!({"kind": "progress", "measures": ["v"]}))
                .contains("needs a goal")
        );
        assert!(
            refused(serde_json::json!({
                "kind": "progress", "measures": ["v", "g"], "goal": 5
            }))
            .contains("exactly one value")
        );
        assert!(
            refused(serde_json::json!({
                "kind": "progress", "measures": ["v"], "goal": {"measure": "g"}
            }))
            .contains("second of exactly two measures")
        );
        assert!(
            refused(serde_json::json!({
                "kind": "progress", "measures": ["v", "g"], "goal": {"measure": "x"}
            }))
            .contains("second of exactly two measures")
        );
        assert!(
            check_fields(&input(serde_json::json!({
                "kind": "progress", "measures": ["v", "g"], "goal": {"measure": "g"}
            })))
            .is_ok()
        );
        assert!(
            refused(serde_json::json!({
                "kind": "progress", "dimension": "p", "measures": ["v"], "goal": 5
            }))
            .contains("no category")
        );
    }

    #[test]
    fn bands_are_one_to_three_finite_limits_going_up() {
        let bullet = |bands: serde_json::Value| serde_json::json!({"kind": "bullet", "dimension": "p", "measures": ["v"], "bands": bands});
        assert!(check_fields(&input(bullet(serde_json::json!([10])))).is_ok());
        assert!(check_fields(&input(bullet(serde_json::json!([10, 20, 30])))).is_ok());
        assert!(refused(bullet(serde_json::json!([1, 2, 3, 4]))).contains("1 to 3 bands"));
        assert!(refused(bullet(serde_json::json!([]))).contains("1 to 3 bands"));
        assert!(refused(bullet(serde_json::json!([30, 20]))).contains("must go up"));
        assert!(refused(bullet(serde_json::json!([20, 20]))).contains("must go up"));
    }

    #[test]
    fn a_histogram_takes_a_width_or_a_count_never_both_and_counts_at_most_a_hundred() {
        let hist = |h: serde_json::Value| serde_json::json!({"kind": "histogram", "dimension": "price", "histogram": h});
        assert!(
            check_fields(&input(serde_json::json!({
                "kind": "histogram", "dimension": "price"
            })))
            .is_ok()
        );
        assert!(check_fields(&input(hist(serde_json::json!({"count": 100})))).is_ok());
        assert!(check_fields(&input(hist(serde_json::json!({"width": 2.5})))).is_ok());
        assert!(refused(hist(serde_json::json!({"count": 101}))).contains("1 to 100 ranges"));
        assert!(refused(hist(serde_json::json!({"count": 0}))).contains("1 to 100 ranges"));
        assert!(refused(hist(serde_json::json!({"count": 2.5}))).contains("1 to 100 ranges"));
        assert!(refused(hist(serde_json::json!({"count": 5, "width": 2}))).contains("not both"));
        assert!(refused(hist(serde_json::json!({"width": 0}))).contains("width must be between"));
        assert!(
            refused(serde_json::json!({
                "kind": "histogram", "dimension": "price", "measures": ["v"]
            }))
            .contains("takes no measure")
        );
        assert!(refused(serde_json::json!({"kind": "histogram"})).contains("number column"));
    }

    #[test]
    fn the_modes_of_a_histogram_are_a_width_a_count_or_about_twenty_ranges() {
        let mode = |h: HistogramBins| h.mode().unwrap();
        assert_eq!(mode(HistogramBins::default()), BinMode::Nearest(20));
        assert_eq!(
            mode(HistogramBins {
                width: None,
                count: Some(10)
            }),
            BinMode::Ranges(10)
        );
        assert_eq!(
            mode(HistogramBins {
                width: Some(0.5),
                count: None
            }),
            BinMode::Width(0.5)
        );
    }

    #[test]
    fn a_card_needs_an_address_and_an_image_a_short_description() {
        assert!(
            check_fields(&input(serde_json::json!({
                "kind": "image", "url": "https://img.example/a.png", "alt": "A logo"
            })))
            .is_ok()
        );
        assert!(
            check_fields(&input(serde_json::json!({
                "kind": "web", "url": "https://status.example/"
            })))
            .is_ok()
        );
        assert!(refused(serde_json::json!({"kind": "web"})).contains("needs an address"));
        assert!(
            refused(serde_json::json!({"kind": "image", "url": "https://img.example/a.png"}))
                .contains("short description")
        );
        let long = "a".repeat(201);
        assert!(
            refused(serde_json::json!({
                "kind": "image", "url": "https://img.example/a.png", "alt": long
            }))
            .contains("at most 200")
        );
        // A space or a control character in the address is the fixed sentence.
        assert_eq!(
            refused(serde_json::json!({"kind": "web", "url": "https://a.example/ x"})),
            CONTENT_SITE_REFUSED
        );
    }

    #[test]
    fn the_address_must_be_https_on_a_listed_site_and_the_sentence_never_echoes_it() {
        let sites = vec!["https://img.example".to_owned()];
        let card = |url: &str| input(serde_json::json!({"kind": "image", "url": url, "alt": "x"}));
        assert!(check_content_url(&card("https://img.example/a/b.png?x=1"), &sites).is_ok());
        assert!(check_content_url(&card("https://IMG.example/a.png"), &sites).is_ok());
        for bad in [
            "http://img.example/a.png",
            "https://other.example/a.png",
            "https://img.example.evil.example/a.png",
            "https://img.example@evil.example/a.png",
            "https://img.example:8443/a.png",
            "javascript:alert(1)",
            "data:image/png;base64,AAAA",
            "//img.example/a.png",
            "",
        ] {
            let err = check_content_url(&card(bad), &sites).unwrap_err();
            assert_eq!(err, CONTENT_SITE_REFUSED, "{bad}");
            assert!(bad.is_empty() || !err.contains(bad));
        }
        assert!(check_content_url(&card("https://img.example/a.png"), &[]).is_err());
        // Other kinds are not touched.
        assert!(
            check_content_url(
                &input(serde_json::json!({"kind": "text", "text": "x"})),
                &[]
            )
            .is_ok()
        );
    }

    // ── the statements ──────────────────────────────────────────────────

    use crate::builder::{ReadContext, RelationColumns, sql_with_filters_report};
    use crate::filters::{ColumnKind, FilterDef};
    use crate::grain::TimeContext;
    use crate::store::StoredChartSpec;
    use std::collections::HashMap;

    fn columns() -> HashMap<String, RelationColumns> {
        let cols: RelationColumns = [
            ("province", ColumnKind::Text),
            ("price", ColumnKind::Number),
            ("visitors", ColumnKind::Number),
            ("target", ColumnKind::Number),
        ]
        .into_iter()
        .map(|(n, k)| (n.to_owned(), k))
        .collect();
        HashMap::from([("mart_x".to_owned(), cols)])
    }

    /// A stored chart of `kind` whose definition is `extra` laid over a base.
    fn stored(kind: ChartKind, extra: &serde_json::Value) -> StoredChartSpec {
        let mut spec = StoredChartSpec::for_test(kind, "mart_x", "", &[], "SELECT 1".to_owned());
        let mut def = serde_json::to_value(&spec.def).unwrap();
        def.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        spec.def = serde_json::from_value(def).unwrap();
        spec
    }

    fn built(spec: &StoredChartSpec, filters: &[FilterDef]) -> crate::builder::FilteredSql {
        let time = TimeContext::default();
        sql_with_filters_report(spec, &[], filters, &columns(), &ReadContext::new(&time))
    }

    fn filter(json: &str) -> FilterDef {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn a_histogram_counts_rows_per_range_with_about_twenty_ranges_unless_told_otherwise() {
        let auto = stored(
            ChartKind::Histogram,
            &serde_json::json!({"dimension": "price"}),
        );
        let got = built(&auto, &[]);
        assert!(
            got.sql
                .starts_with("SELECT price, count() AS __count FROM ("),
            "{}",
            got.sql
        );
        assert!(
            got.sql.contains("GROUP BY price ORDER BY price LIMIT 100"),
            "{}",
            got.sql
        );
        // Aims at the nice width nearest 20 ranges (BI-10's own automatic bins
        // aim at ten); BI-18 part C review fix (SHOULD-FIX) R21 changed this from
        // "the smallest width giving at most 20", which gave 12 ranges.
        assert!(got.sql.contains("log10((mx - mn) / 20)"), "{}", got.sql);
        assert!(got.sql.contains("arraySort(c ->"), "{}", got.sql);
        assert!(!got.sql.contains("<= 20, 1 * p"), "{}", got.sql);
        assert_eq!(
            got.bins.len(),
            1,
            "the tile gets the width, minimum and maximum"
        );
        assert_eq!(got.bins[0].column, "price");

        let count = stored(
            ChartKind::Histogram,
            &serde_json::json!({"dimension": "price", "histogram": {"count": 10}}),
        );
        // A count is honoured, not "at most" (R21).
        assert!(built(&count, &[]).sql.contains("(mx - mn) / 10 AS raw"));
        assert!(!built(&count, &[]).sql.contains("<= 10, 1 * p"));
        let seven = stored(
            ChartKind::Histogram,
            &serde_json::json!({"dimension": "price", "histogram": {"count": 7}}),
        );
        assert!(built(&seven, &[]).sql.contains("(mx - mn) / 7 AS raw"));

        let width = stored(
            ChartKind::Histogram,
            &serde_json::json!({"dimension": "price", "histogram": {"width": 2.5}}),
        );
        let got = built(&width, &[]);
        assert!(got.sql.contains("2.5 AS w"), "{}", got.sql);
        assert!(
            !got.sql.contains("multiIf"),
            "a width asked for is not searched: {}",
            got.sql
        );
    }

    #[test]
    fn a_histograms_filters_sit_under_its_bins_on_the_raw_rows() {
        let spec = stored(
            ChartKind::Histogram,
            &serde_json::json!({"dimension": "price"}),
        );
        let got = built(
            &spec,
            &[filter(r#"{"column":"province","values":["Bali"]}"#)],
        );
        assert!(got.sql.contains("province IN ('Bali')"), "{}", got.sql);
        // The range is read over the same filtered rows the bins are cut from.
        assert_eq!(
            got.sql.matches("province IN ('Bali')").count(),
            2,
            "{}",
            got.sql
        );
        assert!(
            got.bins[0].sql.contains("province IN ('Bali')"),
            "{}",
            got.bins[0].sql
        );
    }

    #[test]
    fn a_progress_bar_reads_its_value_and_a_goal_that_is_a_value_in_one_row() {
        let fixed = stored(
            ChartKind::Progress,
            &serde_json::json!({"measures": ["visitors"], "aggregate": "sum", "goal": 1_000_000}),
        );
        assert_eq!(
            built(
                &fixed,
                &[filter(r#"{"column":"province","values":["Bali"]}"#)]
            )
            .sql,
            "SELECT round(sum(visitors)) AS v FROM serving.mart_x WHERE province IN ('Bali')"
        );
        let value = stored(
            ChartKind::Progress,
            &serde_json::json!({
                "measures": ["visitors", "target"], "aggregate": "sum",
                "goal": {"measure": "target"}
            }),
        );
        assert_eq!(
            built(
                &value,
                &[filter(r#"{"column":"province","values":["Bali"]}"#)]
            )
            .sql,
            "SELECT __m0 AS v, __m1 AS g FROM (SELECT round(sum(visitors)) AS __m0, \
             round(sum(target)) AS __m1 FROM serving.mart_x WHERE province IN ('Bali'))"
        );
    }

    #[test]
    fn a_progress_value_named_like_a_result_column_is_not_read_as_its_own_alias() {
        // A filter on a column called `v` or `g` would read the alias.
        let spec = stored(
            ChartKind::Progress,
            &serde_json::json!({"measures": ["visitors"], "aggregate": "sum", "goal": 5}),
        );
        let mut cols = columns();
        cols.get_mut("mart_x")
            .unwrap()
            .insert("v".to_owned(), ColumnKind::Number);
        let time = TimeContext::default();
        let got = sql_with_filters_report(
            &spec,
            &[],
            &[filter(r#"{"column":"v","op":"between","min":"1"}"#)],
            &cols,
            &ReadContext::new(&time),
        );
        // The filter on `v` is moved under the aggregate, onto the table's column.
        assert_eq!(
            got.sql,
            "SELECT round(sum(visitors)) AS v FROM (SELECT * FROM serving.mart_x WHERE v >= 1) AS flt"
        );
    }

    #[test]
    fn a_bullet_is_one_row_without_a_category_and_one_row_per_category_with_one() {
        let single = stored(
            ChartKind::Bullet,
            &serde_json::json!({
                "measures": ["visitors", "target"], "aggregate": "sum",
                "goal": {"measure": "target"}, "bands": [50, 80, 100]
            }),
        );
        assert!(
            built(
                &single,
                &[filter(r#"{"column":"price","op":"between","min":"1"}"#)]
            )
            .sql
            .starts_with("SELECT __m0 AS v, __m1 AS g FROM (")
        );
        let by_province = stored(
            ChartKind::Bullet,
            &serde_json::json!({
                "dimension": "province", "measures": ["visitors"], "aggregate": "sum",
                "goal": 100, "bands": [50, 80], "limit": 20, "order": "desc"
            }),
        );
        assert_eq!(
            built(
                &by_province,
                &[filter(r#"{"column":"price","op":"between","min":"1"}"#)]
            )
            .sql
            .split(" FROM ")
            .next(),
            Some("SELECT province, round(sum(visitors)) AS visitors")
        );
        let with_goal_value = stored(
            ChartKind::Bullet,
            &serde_json::json!({
                "dimension": "province", "measures": ["visitors", "target"], "aggregate": "sum",
                "goal": {"measure": "target"}, "limit": 20, "order": "desc"
            }),
        );
        assert!(
            built(&with_goal_value, &[filter(r#"{"column":"price","op":"between","min":"1"}"#)])
                .sql
                .starts_with("SELECT province, round(sum(visitors)) AS visitors, round(sum(target)) AS target FROM serving.mart_x WHERE")
        );
    }

    #[test]
    fn an_image_or_web_card_has_no_statement_and_takes_no_filter() {
        for kind in [ChartKind::Image, ChartKind::Web] {
            let spec = stored(
                kind,
                &serde_json::json!({"url": "https://img.example/a.png", "alt": "x"}),
            );
            assert!(kind.is_static());
            let got = built(
                &spec,
                &[filter(r#"{"column":"province","values":["Bali"]}"#)],
            );
            assert_eq!(got.sql, "");
            assert!(got.skipped.is_empty() && got.bins.is_empty());
        }
    }

    // ── the Gantt chart (BI-16 part C1) ─────────────────────────────────

    fn gantt(extra: &serde_json::Value) -> serde_json::Value {
        let mut base = serde_json::json!({
            "kind": "gantt", "dimension": "task", "start": "begins", "end": "ends",
        });
        base.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        base
    }

    #[test]
    fn a_gantt_chart_needs_its_task_start_and_end_and_nothing_to_aggregate() {
        check_fields(&input(gantt(&serde_json::json!({})))).unwrap();
        check_fields(&input(gantt(
            &serde_json::json!({"breakdown": "team", "progress": "done"}),
        )))
        .unwrap();
        for (extra, why) in [
            (serde_json::json!({"dimension": ""}), "task column"),
            (
                serde_json::json!({"end": null}),
                "start column and an end column",
            ),
            (
                serde_json::json!({"start": " "}),
                "start column and an end column",
            ),
            (
                serde_json::json!({"end": "begins"}),
                "two different columns",
            ),
            (serde_json::json!({"progress": ""}), "progress column"),
            (serde_json::json!({"measures": ["visitors"]}), "no measure"),
            (serde_json::json!({"aggregate": "sum"}), "no aggregate"),
            (
                serde_json::json!({"limit": 5}),
                "no aggregate, limit or order",
            ),
            (serde_json::json!({"grain": "month"}), "no time grain"),
        ] {
            let e = refused(gantt(&extra));
            assert!(e.contains(why), "{extra}: {e}");
        }
        // The fields are refused on any other kind.
        let e = refused(serde_json::json!({"kind": "bar", "dimension": "p", "start": "s"}));
        assert!(
            e.contains("start, end and progress is only for a Gantt chart"),
            "{e}"
        );
    }

    #[test]
    fn a_gantt_chart_writes_back_as_it_was_read_and_other_charts_write_none() {
        let wire = serde_json::json!({
            "title": "t", "mart": "m", "kind": "gantt", "dimension": "task", "breakdown": "team",
            "start": "begins", "end": "ends", "progress": "done", "todayLine": false,
        });
        let chart: ChartInput = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(chart.style.today_line, Some(false));
        let back = serde_json::to_value(&chart).unwrap();
        for key in ["start", "end", "progress", "todayLine", "breakdown"] {
            assert_eq!(back[key], wire[key], "{key}");
        }
        let bar = serde_json::to_value(input(serde_json::json!({"kind": "bar"}))).unwrap();
        for key in ["start", "end", "progress", "todayLine"] {
            assert!(bar.get(key).is_none(), "{key}");
        }
        let e = crate::style::check_style(&input(
            serde_json::json!({"kind": "bar", "todayLine": false}),
        ))
        .unwrap_err();
        assert!(
            e.contains("the today line is only for a Gantt chart"),
            "{e}"
        );
        crate::style::check_style(&chart).unwrap();
    }

    fn gantt_columns() -> HashMap<String, RelationColumns> {
        let cols: RelationColumns = [
            ("task", ColumnKind::Text),
            ("team", ColumnKind::Text),
            ("begins", ColumnKind::Date),
            ("ends", ColumnKind::DateTime),
            ("done", ColumnKind::Number),
            ("province", ColumnKind::Text),
        ]
        .into_iter()
        .map(|(n, k)| (n.to_owned(), k))
        .collect();
        HashMap::from([("mart_x".to_owned(), cols)])
    }

    #[test]
    fn the_gantt_statement_keeps_the_first_1001_tasks_and_counts_the_reversed_rows() {
        let spec = stored(
            ChartKind::Gantt,
            &gantt(&serde_json::json!({"breakdown": "team", "progress": "done"})),
        );
        let time = TimeContext::default();
        let got = sql_with_filters_report(
            &spec,
            &[],
            &[filter(r#"{"column":"province","values":["Bali"]}"#)],
            &gantt_columns(),
            &ReadContext::new(&time),
        );
        let sql = &got.sql;
        assert!(
            sql.starts_with(
                "SELECT __t AS `task`, __g AS `group`, min(__s) AS `start`, max(__e) AS `end`, avg(__p) AS `progress` FROM (SELECT task AS __t, team AS __g, begins AS __s, toTimeZone(ends, "
            ),
            "{sql}"
        );
        // The dashboard filter sits under the rename, on the raw rows; reversed rows are left out; first 1,001 by start.
        assert!(
            sql.contains("FROM serving.mart_x WHERE province IN ('Bali')) WHERE __e >= __s"),
            "{sql}"
        );
        assert!(
            sql.contains("GROUP BY __t, __g ORDER BY min(__s), __t LIMIT 1001"),
            "{sql}"
        );
        let Some(crate::builder::TableRead::Reversed { count_sql }) = got.table else {
            panic!("a Gantt chart carries the count of its reversed rows");
        };
        assert!(count_sql.starts_with("SELECT count() AS n FROM serving.mart_x WHERE province IN ('Bali') AND toTimeZone(ends, "), "{count_sql}");
        assert!(count_sql.contains(") < begins"), "{count_sql}");

        // Without a group the group column is the empty string, and without progress there is none.
        let plain = stored(ChartKind::Gantt, &gantt(&serde_json::json!({})));
        let got =
            sql_with_filters_report(&plain, &[], &[], &gantt_columns(), &ReadContext::new(&time));
        assert!(got.sql.contains("'' AS __g"), "{}", got.sql);
        assert!(!got.sql.contains("progress"), "{}", got.sql);
    }
}
