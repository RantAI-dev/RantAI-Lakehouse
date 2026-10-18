//! How a chart looks (`BI-17` part A, decisions D1 and D2 of
//! `docs/superpowers/plans/2026-10-15-bi-17a-chart-formatting.md`): number and
//! date formats, colours, axes, data labels, notes, tooltip fields and
//! conditional colours.
//!
//! # Why one flattened block
//!
//! Everything here is presentation. It changes no statement and no result, so
//! it lives in the same `spec_json` envelope as the rest of the definition (no
//! `ClickHouse` DDL) and is flattened into [`ChartInput`] the way
//! [`crate::tables::TableFields`] and [`crate::shape::ShapeFields`] are. Every
//! field is absent on a chart saved before this existed and is left out of the
//! stored JSON again when it is empty, so such a chart reads and writes back
//! exactly as it was (tested below with a fixture of today's JSON).
//!
//! # Why the checks are here and not in the console
//!
//! The console is not the only writer: the assistant's tools (`format_chart`,
//! `AI-8`) and any API client save the same fields. One validation, run where
//! the rest of a definition is validated, gives every writer the same plain
//! sentence. A field a kind does not take is refused, the way
//! [`crate::tables`] refuses `compare` on a non-KPI, so it is never stored and
//! silently ignored.
//!
//! The renderer lives in the console (`src/features/dashboards/chart-option.ts`
//! and `src/lib/value-format.ts`); this module only says what may be saved.

#![allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference to the field"
)]

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::formatting::is_hex_color;
use crate::specs::ChartKind;
use crate::store::ChartInput;

/// Most digits after the point (feature page, "Format each value").
pub const MAX_DECIMALS: u8 = 10;
/// Most characters of a prefix or a suffix (feature decision 5).
pub const MAX_AFFIX_LEN: usize = 16;
/// Most characters of an axis title.
pub const MAX_AXIS_TITLE_LEN: usize = 60;
/// Most characters of a note (feature decision 11).
pub const MAX_NOTE_LEN: usize = 140;
/// Most notes on a chart (feature decision 10).
pub const MAX_ANNOTATIONS: usize = 20;
/// Most conditional rules on a chart (feature decision 10).
pub const MAX_CONDITIONAL_RULES: usize = 20;
/// Most per-value formats, series colours and tooltip fields on a chart: a
/// bound on stored size, well above what a chart shows.
pub const MAX_ENTRIES: usize = 50;
/// Longest name of a measure, column or series used as a key.
const MAX_KEY_LEN: usize = 120;

/// The palettes the console ships. `company` (the deployment's own, set in
/// Admin > Settings) is also accepted; whether one is set is the console's
/// to say, it falls back to the default when none is.
pub const BUILTIN_PALETTES: [&str; 5] = [
    "default",
    "bright",
    "muted",
    "sequential-blue",
    "diverging-red-blue",
];

/// The ISO 4217 codes of currencies in use. A code outside the list is
/// refused at save (feature decision 4); `Intl.NumberFormat` shows any of them.
const CURRENCIES: &[&str] = &[
    "AED", "AFN", "ALL", "AMD", "ANG", "AOA", "ARS", "AUD", "AWG", "AZN", "BAM", "BBD", "BDT",
    "BGN", "BHD", "BIF", "BMD", "BND", "BOB", "BRL", "BSD", "BTN", "BWP", "BYN", "BZD", "CAD",
    "CDF", "CHF", "CLP", "CNY", "COP", "CRC", "CUP", "CVE", "CZK", "DJF", "DKK", "DOP", "DZD",
    "EGP", "ERN", "ETB", "EUR", "FJD", "FKP", "GBP", "GEL", "GHS", "GIP", "GMD", "GNF", "GTQ",
    "GYD", "HKD", "HNL", "HTG", "HUF", "IDR", "ILS", "INR", "IQD", "IRR", "ISK", "JMD", "JOD",
    "JPY", "KES", "KGS", "KHR", "KMF", "KPW", "KRW", "KWD", "KYD", "KZT", "LAK", "LBP", "LKR",
    "LRD", "LSL", "LYD", "MAD", "MDL", "MGA", "MKD", "MMK", "MNT", "MOP", "MRU", "MUR", "MVR",
    "MWK", "MXN", "MYR", "MZN", "NAD", "NGN", "NIO", "NOK", "NPR", "NZD", "OMR", "PAB", "PEN",
    "PGK", "PHP", "PKR", "PLN", "PYG", "QAR", "RON", "RSD", "RUB", "RWF", "SAR", "SBD", "SCR",
    "SDG", "SEK", "SGD", "SHP", "SLE", "SOS", "SRD", "SSP", "STN", "SVC", "SYP", "SZL", "THB",
    "TJS", "TMT", "TND", "TOP", "TRY", "TTD", "TWD", "TZS", "UAH", "UGX", "USD", "UYU", "UZS",
    "VES", "VND", "VUV", "WST", "XAF", "XCD", "XOF", "XPF", "YER", "ZAR", "ZMW", "ZWG",
];

/// Whether `code` is an ISO 4217 code in the list (upper case).
#[must_use]
pub fn is_currency(code: &str) -> bool {
    CURRENCIES.binary_search(&code).is_ok()
}

/// How a number is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FormatStyle {
    /// What the console showed before this existed.
    #[default]
    Auto,
    /// A plain number.
    Number,
    /// A fraction times 100 with a `%` (0.125 is 12.5 %).
    Percent,
    /// An amount of money; needs [`ValueFormat::currency`].
    Currency,
    /// `1.23E6`.
    Scientific,
    /// `1.2 K`, `3.4 M`, `B`, `T`, in every locale (feature decision 3); with
    /// a [`ValueFormat::currency`] an amount, "Rp 1,2 M".
    Compact,
}

/// Whether a currency is shown as its symbol or its code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CurrencyDisplay {
    /// `Rp`, `$`.
    #[default]
    Symbol,
    /// `IDR`, `USD`.
    Code,
}

fn yes() -> bool {
    true
}

fn is_yes(b: &bool) -> bool {
    *b
}

/// The format of one value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValueFormat {
    /// The style.
    #[serde(default)]
    pub style: FormatStyle,
    /// Digits after the point, 0 to [`MAX_DECIMALS`]; absent means the style's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decimals: Option<u8>,
    /// Group thousands (`1,234` or `1.234` by the deployment's locale).
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub thousands: bool,
    /// An ISO 4217 code, upper case; with the `currency` or `compact` style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    /// Symbol or code.
    #[serde(default, skip_serializing_if = "is_default_display")]
    pub currency_display: CurrencyDisplay,
    /// Text before the number, at most [`MAX_AFFIX_LEN`] characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    /// Text after the number, at most [`MAX_AFFIX_LEN`] characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suffix: Option<String>,
}

fn is_default_display(d: &CurrencyDisplay) -> bool {
    *d == CurrencyDisplay::Symbol
}

impl Default for ValueFormat {
    fn default() -> Self {
        Self {
            style: FormatStyle::Auto,
            decimals: None,
            thousands: true,
            currency: None,
            currency_display: CurrencyDisplay::Symbol,
            prefix: None,
            suffix: None,
        }
    }
}

/// How a date is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DateStyle {
    /// What the console showed before this existed.
    #[default]
    Auto,
    /// `15/10/26`.
    Short,
    /// `15 Oct 2026`.
    Medium,
    /// `15 October 2026`.
    Long,
    /// `2026-10-15`.
    Iso,
    /// `October 2026`.
    MonthYear,
}

/// The format of a date or timestamp.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DateFormat {
    /// The style.
    #[serde(default)]
    pub style: DateStyle,
    /// 12-hour (`true`) or 24-hour (`false`) time; absent follows the locale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hour12: Option<bool>,
    /// Short month and day names.
    #[serde(default, skip_serializing_if = "is_false")]
    pub abbreviate: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Colours of a chart.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChartColors {
    /// A built-in palette id ([`BUILTIN_PALETTES`]) or `company`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<String>,
    /// Series name to `#rrggbb`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub series: BTreeMap<String, String>,
    /// Colour by value: two or three `#rrggbb` stops over the shown minimum
    /// and maximum (feature decision 7).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by_value: Option<Vec<String>>,
}

/// One axis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AxisStyle {
    /// Title, at most [`MAX_AXIS_TITLE_LEN`] characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Draw the axis.
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub show: bool,
    /// Lowest value shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// Highest value shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// Logarithmic scale (feature decision 8).
    #[serde(default, skip_serializing_if = "is_false")]
    pub log: bool,
    /// Label rotation in degrees: 0, 45 or 90.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotate: Option<u16>,
}

impl Default for AxisStyle {
    fn default() -> Self {
        Self {
            title: None,
            show: true,
            min: None,
            max: None,
            log: false,
            rotate: None,
        }
    }
}

impl AxisStyle {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// The axes of a chart.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Axes {
    /// The category (or, for a scatter, the X value) axis.
    #[serde(default, skip_serializing_if = "AxisStyle::is_default")]
    pub x: AxisStyle,
    /// The value axis.
    #[serde(default, skip_serializing_if = "AxisStyle::is_default")]
    pub y: AxisStyle,
    /// The second value axis, on the right.
    #[serde(default, skip_serializing_if = "AxisStyle::is_default")]
    pub y2: AxisStyle,
    /// The values drawn against the second axis, by result name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub right_values: Vec<String>,
}

/// Values written on bars and points.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataLabels {
    /// Write them.
    #[serde(default)]
    pub show: bool,
}

/// A short note pinned to a point of a chart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Annotation {
    /// The category or date value the note is pinned to, as the result shows it.
    pub x: String,
    /// On a chart with several series, which one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series: Option<String>,
    /// Plain text, at most [`MAX_NOTE_LEN`] characters.
    pub text: String,
}

/// The fields a tooltip shows.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TooltipFields {
    /// Result names the chart already returns.
    #[serde(default)]
    pub fields: Vec<String>,
}

/// Comparison operators of a conditional rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompareOp {
    /// Below.
    Lt,
    /// Below or equal.
    Le,
    /// Above.
    Gt,
    /// Above or equal.
    Ge,
    /// Equal.
    Eq,
    /// Not equal.
    Ne,
}

/// What a conditional rule tests.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Rule {
    /// A number against a value.
    Compare {
        /// The operator.
        op: CompareOp,
        /// The value.
        value: f64,
    },
    /// A number within `from..=to`.
    Between {
        /// Lowest.
        from: f64,
        /// Highest.
        to: f64,
    },
    /// Text that contains `text`, ignoring case.
    Contains {
        /// The text.
        text: String,
    },
    /// Two or three colours over the shown minimum and maximum.
    Range {
        /// `#rrggbb` stops.
        colors: Vec<String>,
    },
}

/// What a rule colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CondTarget {
    /// The cell's background.
    #[default]
    Cell,
    /// The text.
    Text,
}

/// Colour a table cell or a KPI value by its value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CondRule {
    /// The column (or, for a KPI, the value) it reads.
    pub column: String,
    /// The test.
    pub rule: Rule,
    /// `#rrggbb` when the test holds; a range carries its own colours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Cell background or text.
    #[serde(default, skip_serializing_if = "is_cell")]
    pub target: CondTarget,
}

fn is_cell(t: &CondTarget) -> bool {
    *t == CondTarget::Cell
}

/// Everything `BI-17` part A adds to a chart's definition.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleFields {
    /// The chart's default number format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<ValueFormat>,
    /// Per-value formats, by result name; they replace [`Self::format`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub value_formats: BTreeMap<String, ValueFormat>,
    /// How dates are written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_format: Option<DateFormat>,
    /// Colours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colors: Option<ChartColors>,
    /// Axes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axes: Option<Axes>,
    /// Data labels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub labels: Option<DataLabels>,
    /// Notes pinned to points.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    /// Tooltip fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<TooltipFields>,
    /// Conditional colours of a table, a pivot or a KPI.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditional: Vec<CondRule>,
}

impl StyleFields {
    /// Whether any field is set.
    #[must_use]
    pub fn is_used(&self) -> bool {
        *self != Self::default()
    }
}

fn plain_text(s: &str, what: &str, max: usize) -> Result<(), String> {
    if s.trim().is_empty() || s.chars().count() > max || s.chars().any(char::is_control) {
        return Err(format!("{what} must be 1 to {max} characters on one line."));
    }
    Ok(())
}

fn affix(s: Option<&String>, what: &str) -> Result<(), String> {
    match s {
        Some(s) if s.chars().count() > MAX_AFFIX_LEN || s.chars().any(char::is_control) => Err(
            format!("{what} is at most {MAX_AFFIX_LEN} characters on one line."),
        ),
        _ => Ok(()),
    }
}

impl ValueFormat {
    /// Check one format; `name` says whose it is in the sentence.
    ///
    /// # Errors
    ///
    /// The sentence to show.
    pub fn validate(&self, name: &str) -> Result<(), String> {
        if self.decimals.is_some_and(|d| d > MAX_DECIMALS) {
            return Err(format!("decimals of {name} must be 0 to {MAX_DECIMALS}."));
        }
        match (&self.currency, self.style) {
            (Some(code), _) if !is_currency(code) => {
                return Err(format!(
                    "'{code}' is not a currency code: use an ISO 4217 code such as IDR or USD."
                ));
            }
            (None, FormatStyle::Currency) => {
                return Err(format!(
                    "the currency format of {name} needs a currency code such as IDR."
                ));
            }
            (Some(_), style) if !matches!(style, FormatStyle::Currency | FormatStyle::Compact) => {
                return Err(format!(
                    "a currency code is only for the currency and compact formats ({name})."
                ));
            }
            _ => {}
        }
        if self.currency_display != CurrencyDisplay::Symbol && self.currency.is_none() {
            return Err(format!(
                "showing a currency as a code needs a currency code ({name})."
            ));
        }
        affix(self.prefix.as_ref(), &format!("the prefix of {name}"))?;
        affix(self.suffix.as_ref(), &format!("the suffix of {name}"))
    }
}

fn key_ok(key: &str, what: &str) -> Result<(), String> {
    if key.is_empty() || key.chars().count() > MAX_KEY_LEN || key.chars().any(char::is_control) {
        return Err(format!("{what} has an invalid name."));
    }
    Ok(())
}

fn colour(c: &str, what: &str) -> Result<(), String> {
    if is_hex_color(c) {
        Ok(())
    } else {
        Err(format!("{what} must be a colour like #1a2b3c."))
    }
}

/// Kinds with a Cartesian value axis the axis options apply to.
fn takes_axes(kind: ChartKind) -> bool {
    matches!(
        kind,
        ChartKind::Bar
            | ChartKind::Hbar
            | ChartKind::Line
            | ChartKind::Area
            | ChartKind::Stacked
            | ChartKind::Combo
            | ChartKind::Scatter
            | ChartKind::Bubble
            | ChartKind::Waterfall
            | ChartKind::Boxplot
    )
}

/// Kinds that can draw a second value axis.
fn takes_second_axis(kind: ChartKind) -> bool {
    matches!(
        kind,
        ChartKind::Bar | ChartKind::Line | ChartKind::Area | ChartKind::Combo
    )
}

/// Kinds whose tooltip lists every value at one category (axis trigger), the
/// ones a tooltip can be limited to chosen fields.
fn takes_tooltip_fields(kind: ChartKind) -> bool {
    matches!(
        kind,
        ChartKind::Bar
            | ChartKind::Hbar
            | ChartKind::Line
            | ChartKind::Area
            | ChartKind::Stacked
            | ChartKind::Combo
    )
}

/// Kinds a notes can be pinned to: a category or date axis.
fn takes_notes(kind: ChartKind) -> bool {
    matches!(
        kind,
        ChartKind::Bar
            | ChartKind::Hbar
            | ChartKind::Line
            | ChartKind::Area
            | ChartKind::Stacked
            | ChartKind::Combo
            | ChartKind::Waterfall
    )
}

/// Kinds colour by value applies to: one series, one colour per mark
/// (feature decision 7).
fn takes_colour_by_value(kind: ChartKind) -> bool {
    matches!(
        kind,
        ChartKind::Bar | ChartKind::Hbar | ChartKind::Pie | ChartKind::Treemap | ChartKind::Geomap
    )
}

/// Kinds drawn as a chart (not a table, a number tile or a static tile).
fn is_chart(kind: ChartKind) -> bool {
    !kind.is_static()
        && !matches!(
            kind,
            ChartKind::Table
                | ChartKind::Pivot
                | ChartKind::Kpi
                | ChartKind::Gauge
                | ChartKind::Progress
        )
}

fn takes_conditional(kind: ChartKind) -> bool {
    matches!(kind, ChartKind::Table | ChartKind::Pivot | ChartKind::Kpi)
}

fn check_axis(axis: &AxisStyle, name: &str, log_ok: bool) -> Result<(), String> {
    if let Some(title) = &axis.title {
        plain_text(
            title,
            &format!("the title of the {name} axis"),
            MAX_AXIS_TITLE_LEN,
        )?;
    }
    for bound in [axis.min, axis.max].into_iter().flatten() {
        if !(bound.is_finite() && bound.abs() < 1e15) {
            return Err(format!("the {name} axis limits must be finite numbers."));
        }
    }
    if let (Some(min), Some(max)) = (axis.min, axis.max)
        && min >= max
    {
        return Err(format!(
            "the {name} axis minimum must be below its maximum."
        ));
    }
    if axis.log {
        if !log_ok {
            return Err(format!(
                "the {name} axis of this chart is not a value axis, so it cannot be logarithmic."
            ));
        }
        if axis.min.is_some_and(|m| m <= 0.0) || axis.max.is_some_and(|m| m <= 0.0) {
            return Err(format!(
                "the limits of a logarithmic {name} axis must be above zero."
            ));
        }
    }
    if axis.rotate.is_some_and(|r| ![0, 45, 90].contains(&r)) {
        return Err(format!("label rotation of the {name} axis is 0, 45 or 90."));
    }
    Ok(())
}

fn check_colors(colors: &ChartColors, def: &ChartInput) -> Result<(), String> {
    if let Some(palette) = &colors.palette
        && palette != "company"
        && !BUILTIN_PALETTES.contains(&palette.as_str())
    {
        return Err(format!(
            "unknown palette '{palette}': use {} or company.",
            BUILTIN_PALETTES.join(", ")
        ));
    }
    if colors.series.len() > MAX_ENTRIES {
        return Err(format!("at most {MAX_ENTRIES} series colours."));
    }
    for (name, c) in &colors.series {
        key_ok(name, "a series colour")?;
        colour(c, &format!("the colour of series '{name}'"))?;
    }
    if let Some(stops) = &colors.by_value {
        if !takes_colour_by_value(def.kind) {
            return Err(
                "colour by value is for a bar, horizontal bar, pie, treemap or map chart."
                    .to_owned(),
            );
        }
        if def.breakdown.as_deref().is_some_and(|b| !b.is_empty())
            || def.measures.entries().count() > 1
        {
            return Err(
                "colour by value needs a chart with one value and no breakdown.".to_owned(),
            );
        }
        if !(2..=3).contains(&stops.len()) {
            return Err("colour by value takes two or three colours.".to_owned());
        }
        for c in stops {
            colour(c, "a colour by value")?;
        }
    }
    Ok(())
}

fn check_rule(rule: &CondRule) -> Result<(), String> {
    key_ok(&rule.column, "a conditional rule")?;
    let finite = |v: f64| v.is_finite() && v.abs() < 1e15;
    match &rule.rule {
        Rule::Compare { value, .. } if !finite(*value) => {
            return Err("a conditional rule value must be a finite number.".to_owned());
        }
        Rule::Between { from, to } => {
            if !(finite(*from) && finite(*to)) {
                return Err("a conditional rule range must be finite numbers.".to_owned());
            }
            if from > to {
                return Err("a conditional range starts at its lower value.".to_owned());
            }
        }
        Rule::Contains { text } => plain_text(text, "the text of a conditional rule", 80)?,
        Rule::Range { colors } => {
            if !(2..=3).contains(&colors.len()) {
                return Err("a colour range takes two or three colours.".to_owned());
            }
            for c in colors {
                colour(c, "a colour of a range")?;
            }
            if rule.color.is_some() {
                return Err("a colour range carries its own colours: leave color out.".to_owned());
            }
            return Ok(());
        }
        Rule::Compare { .. } => {}
    }
    match &rule.color {
        Some(c) => colour(c, "the colour of a conditional rule"),
        None => Err("a conditional rule needs a colour.".to_owned()),
    }
}

/// Check the style of a chart: which fields its kind takes, and the shape of
/// each field. Needs neither the relation nor its columns; see
/// [`check_style_names`] for the names.
///
/// # Errors
///
/// The sentence to show the person who saved the chart.
pub fn check_style(def: &ChartInput) -> Result<(), String> {
    let style = &def.style;
    if !style.is_used() {
        return Ok(());
    }
    let kind = def.kind;
    if kind.is_static() {
        return Err("a tile without data has nothing to format.".to_owned());
    }
    let wrong = |field: &str, kinds: &str| format!("{field} is only for {kinds}.");
    if let Some(f) = &style.format {
        f.validate("the chart")?;
    }
    if style.value_formats.len() > MAX_ENTRIES {
        return Err(format!("at most {MAX_ENTRIES} value formats."));
    }
    for (name, f) in &style.value_formats {
        key_ok(name, "a value format")?;
        f.validate(&format!("'{name}'"))?;
    }
    if let Some(colors) = &style.colors {
        if !is_chart(kind) {
            return Err(wrong(
                "colours",
                "charts (use conditional colours for a table or KPI)",
            ));
        }
        check_colors(colors, def)?;
    }
    if let Some(axes) = &style.axes {
        if !takes_axes(kind) {
            return Err(wrong(
                "axes",
                "a bar, line, area, stacked, combo, scatter, bubble, waterfall or box plot chart",
            ));
        }
        let x_is_value = matches!(kind, ChartKind::Scatter | ChartKind::Bubble);
        let y_log_ok = !matches!(kind, ChartKind::Stacked | ChartKind::Waterfall);
        check_axis(&axes.x, "X", x_is_value)?;
        check_axis(&axes.y, "Y", y_log_ok)?;
        check_axis(&axes.y2, "second Y", y_log_ok)?;
        if (!axes.y2.is_default() || !axes.right_values.is_empty()) && !takes_second_axis(kind) {
            return Err(wrong("a second Y axis", "a bar, line, area or combo chart"));
        }
        if axes.right_values.len() > MAX_ENTRIES {
            return Err(format!("at most {MAX_ENTRIES} values on the second axis."));
        }
        for v in &axes.right_values {
            key_ok(v, "a value on the second axis")?;
        }
    }
    if style.labels.is_some() && !is_chart(kind) {
        return Err(wrong("data labels", "charts"));
    }
    if !style.annotations.is_empty() {
        if !takes_notes(kind) {
            return Err(wrong(
                "a note",
                "a bar, line, area, stacked, combo or waterfall chart",
            ));
        }
        if style.annotations.len() > MAX_ANNOTATIONS {
            return Err(format!("a chart has at most {MAX_ANNOTATIONS} notes."));
        }
        for note in &style.annotations {
            key_ok(&note.x, "a note's point")?;
            plain_text(&note.text, "a note", MAX_NOTE_LEN)?;
            if let Some(series) = &note.series {
                key_ok(series, "a note's series")?;
            }
        }
    }
    if let Some(tooltip) = &style.tooltip {
        if !takes_tooltip_fields(kind) {
            return Err(wrong(
                "tooltip fields",
                "a bar, line, area, stacked or combo chart",
            ));
        }
        if tooltip.fields.len() > MAX_ENTRIES {
            return Err(format!("at most {MAX_ENTRIES} tooltip fields."));
        }
        for f in &tooltip.fields {
            key_ok(f, "a tooltip field")?;
        }
    }
    if !style.conditional.is_empty() {
        if !takes_conditional(kind) {
            return Err(wrong("conditional colours", "a table, pivot or KPI"));
        }
        if style.conditional.len() > MAX_CONDITIONAL_RULES {
            return Err(format!(
                "a chart has at most {MAX_CONDITIONAL_RULES} conditional rules."
            ));
        }
        for rule in &style.conditional {
            check_rule(rule)?;
        }
    }
    Ok(())
}

/// Check that every name the style uses is one the chart returns: a column of
/// its relation or a result name of its values. Called where the relation's
/// columns are known (a parameter chart, whose columns change with the
/// parameter, is checked by [`check_style`] only).
///
/// # Errors
///
/// The sentence to show the person who saved the chart.
pub fn check_style_names<S: std::hash::BuildHasher>(
    def: &ChartInput,
    cols: &HashSet<String, S>,
) -> Result<(), String> {
    let style = &def.style;
    if !style.is_used() {
        return Ok(());
    }
    let mut known: HashSet<&str> = cols.iter().map(String::as_str).collect();
    let results = def.measures.result_names();
    known.extend(results.iter().map(String::as_str));
    known.insert(def.dimension.as_str());
    if let Some(b) = &def.breakdown {
        known.insert(b.as_str());
    }
    let check = |name: &str, what: &str| {
        if known.contains(name) {
            Ok(())
        } else {
            Err(format!(
                "{what} names '{name}', which the chart does not return."
            ))
        }
    };
    for name in style.value_formats.keys() {
        check(name, "a value format")?;
    }
    for rule in &style.conditional {
        check(&rule.column, "a conditional rule")?;
    }
    if let Some(axes) = &style.axes {
        for v in &axes.right_values {
            check(v, "the second axis")?;
        }
    }
    if let Some(tooltip) = &style.tooltip {
        for f in &tooltip.fields {
            check(f, "the tooltip")?;
        }
    }
    Ok(())
}

/// Key a chart's series colours by the name the chart draws (`BI-18` part C
/// review fix (BLOCKER) R24). The assistant (and a person) names a measure's
/// colour `sum_visitors`, but the series a tile draws is named by the result
/// column (`visitors` on a SQL source, whatever `y` is), so the colour matched
/// nothing and the bars stayed the palette's. A key that is a measure's column
/// or `<aggregate>_<column>` becomes the `y` name of that measure; any other
/// key (a breakdown value, a name already right) is left as it is, and a key
/// is never moved onto one that is already set.
pub fn key_series_by_drawn_name(def: &mut ChartInput, y: &[String]) {
    let Some(colors) = def.style.colors.as_mut() else {
        return;
    };
    if colors.series.is_empty() {
        return;
    }
    let chart_aggregate = def.aggregate.clone();
    let mut moves: Vec<(String, String)> = Vec::new();
    for (i, (column, own)) in def.measures.entries().enumerate() {
        let Some(drawn) = y.get(i) else { continue };
        let mut aliases = vec![column.to_owned()];
        for aggregate in [own, chart_aggregate.as_deref()].into_iter().flatten() {
            aliases.push(format!("{aggregate}_{column}"));
        }
        for alias in aliases {
            if alias != *drawn {
                moves.push((alias, drawn.clone()));
            }
        }
    }
    for (from, to) in moves {
        if colors.series.contains_key(&to) {
            continue;
        }
        if let Some(colour) = colors.series.remove(&from) {
            colors.series.insert(to, colour);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::{Value, json};

    use super::*;

    fn chart(extra: &Value) -> ChartInput {
        let mut base = json!({
            "title": "t", "mart": "mart_x", "kind": "bar",
            "dimension": "province", "measures": ["visitors"],
        });
        for (k, v) in extra.as_object().unwrap() {
            base[k] = v.clone();
        }
        serde_json::from_value(base).unwrap()
    }

    fn refused(extra: &Value) -> String {
        check_style(&chart(extra)).unwrap_err()
    }

    #[test]
    fn the_currency_list_is_sorted_so_the_lookup_is_exact() {
        assert!(CURRENCIES.windows(2).all(|w| w[0] < w[1]));
        assert!(is_currency("IDR") && is_currency("USD") && is_currency("EUR"));
        assert!(!is_currency("XXX") && !is_currency("idr") && !is_currency("RP"));
    }

    /// Plan T2: a definition stored before this branch has none of the style
    /// fields, and reading and writing it back changes nothing.
    #[test]
    fn a_chart_saved_before_styling_round_trips_unchanged() {
        let stored = json!({
            "title": "Visitors", "subtitle": "per province", "mart": "mart_wisman",
            "kind": "combo", "dimension": "province",
            "measures": ["visitors", {"column": "rooms", "aggregate": "avg"}],
            "breakdown": "origin", "limit": 20, "order": "desc", "span": 2,
            "board": "b1", "grain": "month",
            "referenceLines": [
                {"value": 1000.0, "label": "Goal"},
                {"value": {"param": "target"}},
            ],
            "columnSettings": {"visitors": {"format": "number", "decimals": 2}},
        });
        let input: ChartInput = serde_json::from_value(stored.clone()).unwrap();
        assert!(!input.style.is_used());
        assert_eq!(serde_json::to_value(&input).unwrap(), stored);
        assert!(check_style(&input).is_ok());
    }

    #[test]
    fn every_style_field_round_trips_through_the_stored_json() {
        let full = json!({
            "format": {"style": "currency", "currency": "IDR", "decimals": 2, "prefix": "~"},
            "valueFormats": {"visitors": {"style": "compact", "thousands": false}},
            "dateFormat": {"style": "monthYear", "hour12": false, "abbreviate": true},
            "colors": {"palette": "muted", "series": {"A": "#112233"}},
            "axes": {"y": {"title": "Visitors", "min": 0.0, "max": 10.0, "rotate": 45},
                     "y2": {"show": false}, "rightValues": ["visitors"]},
            "labels": {"show": true},
            "annotations": [{"x": "Bali", "series": "A", "text": "Peak"}],
            "tooltip": {"fields": ["visitors"]},
        });
        let input = chart(&full);
        check_style(&input).unwrap();
        let back = serde_json::to_value(&input).unwrap();
        for (k, v) in full.as_object().unwrap() {
            assert_eq!(&back[k], v, "{k}");
        }
    }

    #[test]
    fn decimals_currency_and_affixes_are_checked() {
        let e = refused(&json!({"format": {"decimals": 11}}));
        assert!(e.contains("0 to 10"), "{e}");
        check_style(&chart(&json!({"format": {"decimals": 10}}))).unwrap();
        let e = refused(&json!({"format": {"style": "currency", "currency": "ZZZ"}}));
        assert!(e.contains("not a currency code"), "{e}");
        let e = refused(&json!({"format": {"style": "currency"}}));
        assert!(e.contains("needs a currency code"), "{e}");
        let e = refused(&json!({"format": {"style": "number", "currency": "IDR"}}));
        assert!(
            e.contains("only for the currency and compact formats"),
            "{e}"
        );
        check_style(&chart(
            &json!({"format": {"style": "compact", "currency": "IDR", "decimals": 2}}),
        ))
        .unwrap();
        let e = refused(&json!({"format": {"prefix": "12345678901234567"}}));
        assert!(e.contains("prefix"), "{e}");
        check_style(&chart(&json!({"format": {"suffix": "1234567890123456"}}))).unwrap();
        let e = refused(&json!({"valueFormats": {"visitors": {"suffix": "a\nb"}}}));
        assert!(e.contains("suffix"), "{e}");
    }

    #[test]
    fn the_palette_and_the_colours_are_checked() {
        let e = refused(&json!({"colors": {"palette": "neon"}}));
        assert!(e.contains("unknown palette"), "{e}");
        check_style(&chart(&json!({"colors": {"palette": "company"}}))).unwrap();
        let e = refused(&json!({"colors": {"series": {"A": "red"}}}));
        assert!(e.contains("#1a2b3c"), "{e}");
        let e = refused(&json!({"colors": {"byValue": ["#112233"]}}));
        assert!(e.contains("two or three"), "{e}");
        check_style(&chart(
            &json!({"colors": {"byValue": ["#112233", "#445566"]}}),
        ))
        .unwrap();
        let e =
            refused(&json!({"colors": {"byValue": ["#112233", "#445566"]}, "breakdown": "origin"}));
        assert!(e.contains("one value and no breakdown"), "{e}");
        let e = refused(&json!({"colors": {"byValue": ["#112233", "#445566"]}, "kind": "line"}));
        assert!(e.contains("colour by value is for"), "{e}");
    }

    #[test]
    fn a_logarithmic_axis_is_allowed_only_on_a_value_axis() {
        check_style(&chart(&json!({"axes": {"y": {"log": true}}}))).unwrap();
        let e = refused(&json!({"axes": {"x": {"log": true}}}));
        assert!(e.contains("not a value axis"), "{e}");
        check_style(&chart(
            &json!({"kind": "scatter", "axes": {"x": {"log": true}}}),
        ))
        .unwrap();
        let e = refused(&json!({"axes": {"y": {"log": true, "min": 0}}}));
        assert!(e.contains("above zero"), "{e}");
        let e = refused(&json!({"axes": {"y": {"min": 5, "max": 5}}}));
        assert!(e.contains("below its maximum"), "{e}");
        let e = refused(&json!({"axes": {"x": {"rotate": 30}}}));
        assert!(e.contains("0, 45 or 90"), "{e}");
        let e = refused(&json!({"axes": {"y": {"title": "x".repeat(61)}}}));
        assert!(e.contains("1 to 60"), "{e}");
    }

    #[test]
    fn a_second_axis_is_for_bar_line_area_and_combo_charts() {
        check_style(&chart(
            &json!({"kind": "combo", "axes": {"rightValues": ["visitors"]}}),
        ))
        .unwrap();
        let e = refused(&json!({"kind": "hbar", "axes": {"rightValues": ["visitors"]}}));
        assert!(e.contains("second Y axis"), "{e}");
        let e = refused(&json!({"kind": "pie", "axes": {"y": {"show": false}}}));
        assert!(e.contains("axes is only for"), "{e}");
    }

    #[test]
    fn notes_are_limited_in_number_and_length_and_to_the_kinds_with_a_category_axis() {
        let note = |n: usize| json!({"x": "Bali", "text": format!("note {n}")});
        let twenty: Vec<Value> = (0..20).map(note).collect();
        check_style(&chart(&json!({"annotations": twenty}))).unwrap();
        let twenty_one: Vec<Value> = (0..21).map(note).collect();
        assert!(refused(&json!({"annotations": twenty_one})).contains("at most 20 notes"));
        let long = json!([{"x": "Bali", "text": "x".repeat(141)}]);
        assert!(refused(&json!({"annotations": long})).contains("1 to 140"));
        let e = refused(&json!({"kind": "pie", "tooltip": {"fields": ["visitors"]}}));
        assert!(e.contains("tooltip fields is only for"), "{e}");
        let e = refused(&json!({"kind": "pie", "annotations": [note(1)]}));
        assert!(e.contains("a note is only for"), "{e}");
    }

    #[test]
    fn conditional_rules_are_for_tables_pivots_and_kpis_only_and_at_most_twenty() {
        let rule = json!({"column": "visitors", "rule": {"type": "compare", "op": "lt", "value": 1000},
                          "color": "#ff0000"});
        let e = refused(&json!({"conditional": [rule]}));
        assert!(
            e.contains("conditional colours is only for a table, pivot or KPI"),
            "{e}"
        );
        for kind in ["table", "pivot", "kpi"] {
            check_style(&chart(&json!({"kind": kind, "conditional": [rule]}))).unwrap();
        }
        let many: Vec<Value> = (0..21).map(|_| rule.clone()).collect();
        let e = refused(&json!({"kind": "table", "conditional": many}));
        assert!(e.contains("at most 20 conditional rules"), "{e}");
        let no_colour = json!({"column": "v", "rule": {"type": "contains", "text": "x"}});
        assert!(
            refused(&json!({"kind": "table", "conditional": [no_colour]}))
                .contains("needs a colour")
        );
        let range =
            json!({"column": "v", "rule": {"type": "range", "colors": ["#000000", "#ffffff"]}});
        check_style(&chart(&json!({"kind": "table", "conditional": [range]}))).unwrap();
        let backwards = json!({"column": "v", "rule": {"type": "between", "from": 5, "to": 1}, "color": "#000000"});
        assert!(
            refused(&json!({"kind": "table", "conditional": [backwards]})).contains("lower value")
        );
    }

    #[test]
    fn a_text_tile_has_nothing_to_format() {
        let e = refused(&json!({"kind": "text", "format": {"decimals": 1}}));
        assert!(e.contains("nothing to format"), "{e}");
    }

    #[test]
    fn colours_are_refused_on_a_table_and_labels_on_a_kpi() {
        assert!(
            refused(&json!({"kind": "table", "colors": {"palette": "muted"}}))
                .contains("colours is only")
        );
        assert!(
            refused(&json!({"kind": "kpi", "labels": {"show": true}}))
                .contains("data labels is only")
        );
    }

    #[test]
    fn names_the_style_uses_must_be_columns_or_results_of_the_chart() {
        let cols: HashSet<String> = ["province", "visitors", "rooms"].map(str::to_owned).into();
        let ok = chart(&json!({"valueFormats": {"visitors": {"style": "number"}},
                               "tooltip": {"fields": ["rooms"]}}));
        check_style_names(&ok, &cols).unwrap();
        let bad = chart(&json!({"valueFormats": {"ghost": {"style": "number"}}}));
        assert!(
            check_style_names(&bad, &cols)
                .unwrap_err()
                .contains("'ghost'")
        );
        let own = chart(
            &json!({"measures": [{"column": "visitors", "aggregate": "avg"}],
                                "valueFormats": {"avg_visitors": {"style": "number"}}}),
        );
        check_style_names(&own, &cols).unwrap();
        let bad_rule = chart(&json!({"kind": "table", "conditional": [
            {"column": "nope", "rule": {"type": "contains", "text": "x"}, "color": "#000000"}]}));
        assert!(check_style_names(&bad_rule, &cols).is_err());
    }

    fn colours_of(extra: serde_json::Value) -> ChartInput {
        let mut def = serde_json::json!({
            "title": "t", "mart": "m", "kind": "hbar", "dimension": "provinsi",
            "measures": ["visitors"], "aggregate": "sum",
        });
        def.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::from_value(def).unwrap()
    }

    /// `BI-18` part C review fix (BLOCKER) R24: the chart of the owner's QA
    /// (`u_be2bc0d2`) drew `visitors`; the colour was saved under `sum_visitors`.
    #[test]
    fn a_series_colour_is_keyed_by_the_name_the_chart_draws() {
        let mut def = colours_of(serde_json::json!({
            "colors": { "series": { "sum_visitors": "#16a34a" } },
            "format": { "style": "currency", "currency": "IDR" },
            "referenceLines": [{ "value": 1000.0, "label": "Target" }],
        }));
        key_series_by_drawn_name(&mut def, &["visitors".to_owned()]);
        let colors = def.style.colors.as_ref().unwrap();
        assert_eq!(
            colors.series.get("visitors").map(String::as_str),
            Some("#16a34a")
        );
        assert!(!colors.series.contains_key("sum_visitors"));
        // The rest of the look is untouched.
        assert!(def.style.format.is_some());
        assert!(def.shape.reference_lines.is_some());
        // A name the chart already draws, a breakdown value and an unknown name stay.
        let mut right = colours_of(serde_json::json!({
            "colors": { "series": { "sum_visitors": "#111111", "Bali": "#222222" } },
        }));
        key_series_by_drawn_name(&mut right, &["sum_visitors".to_owned()]);
        let kept = &right.style.colors.as_ref().unwrap().series;
        assert_eq!(kept.len(), 2);
        assert!(kept.contains_key("sum_visitors") && kept.contains_key("Bali"));
        // The bare column also maps, and a key is never moved onto one already set.
        let mut both = colours_of(serde_json::json!({
            "colors": { "series": { "visitors": "#333333", "sum_visitors": "#444444" } },
        }));
        key_series_by_drawn_name(&mut both, &["visitors".to_owned()]);
        let series = &both.style.colors.as_ref().unwrap().series;
        assert_eq!(series["visitors"], "#333333");
    }
}
