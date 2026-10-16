//! Number bins and coordinate grids (`BI-10`, with `AI-6`): grouping by a
//! number column in ranges, and by the cell of a latitude/longitude grid.
//!
//! # A bin is to a number what a grain is to a date
//!
//! [`crate::grain`] cuts a date into buckets inside an inner relation that
//! carries the bucket under the grouping's own name, with the raw-row filters
//! underneath ([`crate::builder::Relation::Bucketed`] over
//! [`crate::builder::Relation::Filtered`]). A bin does the same for a number,
//! and everything downstream (sort, "Other", steps, the role rewrite) reads
//! the grouping unchanged.
//!
//! The result value of a bin is its **lower edge**, a number, so it sorts and
//! draws like any number. A bin includes its lower edge and excludes its
//! upper edge, except the last bin, which also includes the maximum: the
//! maximum is never alone in a bin of its own (`BI-10-AC1`).
//!
//! # One statement, no pre-read round trip
//!
//! `auto` and `count` need the column's minimum and maximum over the rows the
//! viewer may see and over the filters in force. They are read by a scalar
//! subquery joined into the same statement ([`Range::scalars_sql`]), over the
//! very same filtered relation text as the grouping reads, so:
//!
//! - the edges can never be computed from different rows than the bins hold;
//! - the role rewrite sees the table reference inside the subquery as a plain
//!   table reference and substitutes the masked, row-filtered projection
//!   there too (verified with the real rewrite, plan section 5);
//! - no call site needs a second, asynchronous, read.
//!
//! The tile still needs the width to label `500 – 1,000`, so the builder also
//! returns [`Range::read_sql`], one row `(width, min, max)` over the same
//! relation, which the API runs through the same rewrite
//! (`routes::support::annotate_bins`).
//!
//! # How the width is chosen
//!
//! `auto` aims at about ten bins, `count` at 10, 50 or 100. The width is the
//! smallest of 1, 2, 5, 10, 20, 50 and 100 times the power of ten just below
//! `(max - min) / n` whose bins, aligned to multiples of the width, are at most
//! `n` (alignment can cost one bin over `(max - min) / width`). A range of zero
//! has width 1. Edges are multiples of the width, so `visitors` in bins of 500
//! reads `0, 500, 1000, ...`.
//!
//! # Grid
//!
//! A grid cell is `size` degrees square. [`grid_points_sql`] groups the
//! points of a map by cell and returns one row per cell at the cell's centre
//! with the aggregate and the point count. `auto` picks the largest listed
//! size that gives at least [`GRID_MIN_CELLS`] cells (the smallest otherwise),
//! counted by the engine over the same filtered relation.
//!
//! # Safety of the SQL
//!
//! Columns are [`Ident`]s checked against the relation; the only numbers that
//! reach the text are a count from a closed list and a width the API parsed
//! and range-checked, printed by Rust. Nothing else comes from a caller.

use lakehouse_core::ident::Ident;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::specs::Aggregate;

/// The bin counts a chart may ask for.
pub const COUNTS: [u32; 3] = [10, 50, 100];

/// The bins `auto` aims at.
pub const AUTO_BINS: u32 = 10;

/// Narrowest and widest bin a chart may ask for.
pub const MIN_WIDTH: f64 = 0.000_001;
/// See [`MIN_WIDTH`].
pub const MAX_WIDTH: f64 = 1e12;

/// Fewest cells `auto` accepts for a size.
pub const GRID_MIN_CELLS: u32 = 50;

/// The grid sizes in degrees, largest first.
pub const GRID_SIZES: [f64; 4] = [10.0, 1.0, 0.1, 0.01];

/// How a number is binned, as it is written on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "RawBin")]
pub struct BinSpec {
    /// `auto`, `count` or `width`.
    pub mode: String,
    /// `count`: 10, 50 or 100.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    /// `width`: a positive number.
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::store::serialize_js_number"
    )]
    pub width: Option<f64>,
    /// Names of the fields that were neither a number nor a string. Never
    /// written back; [`BinSpec::mode`] refuses the bin.
    #[serde(skip)]
    pub invalid: Vec<&'static str>,
}

/// `width` is finite whenever it is present (a non-finite one is recorded in
/// `invalid` instead), so equality is reflexive and `Eq` holds.
impl Eq for BinSpec {}

/// The wire shape of [`BinSpec`]: `count` and `width` as loose JSON, because a
/// caller may send either a number or a string (and a decode error carries no
/// text a person could act on).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBin {
    mode: String,
    #[serde(default)]
    count: Option<serde_json::Value>,
    #[serde(default)]
    width: Option<serde_json::Value>,
}

fn loose_number(
    name: &'static str,
    v: Option<serde_json::Value>,
    invalid: &mut Vec<&'static str>,
) -> Option<f64> {
    match v {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::Number(n)) => n.as_f64().filter(|v| v.is_finite()),
        Some(serde_json::Value::String(s)) => s
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .or_else(|| {
                invalid.push(name);
                None
            }),
        Some(_) => {
            invalid.push(name);
            None
        }
    }
}

impl From<RawBin> for BinSpec {
    fn from(raw: RawBin) -> Self {
        let mut invalid = Vec::new();
        let count = loose_number("count", raw.count, &mut invalid);
        let width = loose_number("width", raw.width, &mut invalid);
        Self {
            mode: raw.mode,
            // A count that is not a whole non-negative number is kept as
            // `invalid`, not truncated into another one.
            count: count.and_then(|c| {
                if c.fract() == 0.0 && (0.0..=1e9).contains(&c) {
                    #[allow(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        reason = "checked just above: a whole number between 0 and 1e9"
                    )]
                    Some(c as u32)
                } else {
                    invalid.push("count");
                    None
                }
            }),
            width,
            invalid,
        }
    }
}

/// A checked bin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinMode {
    /// About [`AUTO_BINS`] bins.
    Auto,
    /// At most this many bins (one of [`COUNTS`]).
    Count(u32),
    /// Bins of exactly this width.
    Width(f64),
}

impl BinSpec {
    /// The checked mode.
    ///
    /// # Errors
    ///
    /// The sentence to show the person who chose the bin.
    pub fn mode(&self) -> Result<BinMode, String> {
        if let Some(name) = self.invalid.first() {
            return Err(format!("the bin's {name} must be a number."));
        }
        match self.mode.as_str() {
            "auto" if self.count.is_none() && self.width.is_none() => Ok(BinMode::Auto),
            "auto" => Err("automatic bins take no count or width.".to_owned()),
            "count" => match (self.count, self.width) {
                (Some(c), None) if COUNTS.contains(&c) => Ok(BinMode::Count(c)),
                _ => Err(format!(
                    "bins by count take a count of {}.",
                    COUNTS.map(|c| c.to_string()).join(", ")
                )),
            },
            "width" => match (self.width, self.count) {
                (Some(w), None) if w.is_finite() && (MIN_WIDTH..=MAX_WIDTH).contains(&w) => {
                    Ok(BinMode::Width(w))
                }
                _ => Err(format!(
                    "bins by width take a width between {MIN_WIDTH} and {MAX_WIDTH}."
                )),
            },
            other => Err(format!(
                "invalid bin mode '{other}': use auto, count or width."
            )),
        }
    }
}

/// A number printed the way the engine reads it: no exponent, no trailing
/// noise (Rust's `Display` for `f64` never prints an exponent).
fn literal(v: f64) -> String {
    format!("{v}")
}

/// How a bin's width is found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WidthRule {
    /// A width that was asked for, printed.
    Fixed(String),
    /// The smallest round width giving at most this many bins.
    Nice(u32),
}

impl BinMode {
    /// The rule that finds this mode's width.
    #[must_use]
    pub fn rule(self) -> WidthRule {
        match self {
            Self::Auto => WidthRule::Nice(AUTO_BINS),
            Self::Count(n) => WidthRule::Nice(n),
            Self::Width(w) => WidthRule::Fixed(literal(w)),
        }
    }
}

/// The column a bin reads and the rule for its width: what a relation needs to
/// carry the column's range ([`crate::builder::Relation::Ranged`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Range {
    /// The raw number column.
    pub column: String,
    /// How the width is found.
    pub rule: WidthRule,
}

/// Multiples of the power of ten tried, smallest first.
const NICE: [u32; 7] = [1, 2, 5, 10, 20, 50, 100];

impl Range {
    /// A range over `column` for `mode`.
    #[must_use]
    pub fn new(column: &Ident, mode: BinMode) -> Self {
        Self {
            column: column.to_string(),
            rule: mode.rule(),
        }
    }

    fn mn(&self) -> String {
        format!("__bmn_{}", self.column)
    }

    fn mx(&self) -> String {
        format!("__bmx_{}", self.column)
    }

    fn w(&self) -> String {
        format!("__bw_{}", self.column)
    }

    /// `SELECT mn, mx, w FROM ...` over `base`: the column's minimum and
    /// maximum and the bin width, one row.
    fn stats_sql(&self, base: &str) -> String {
        let c = &self.column;
        let l1 = format!("SELECT min(toFloat64({c})) AS mn, max(toFloat64({c})) AS mx FROM {base}");
        match &self.rule {
            WidthRule::Fixed(w) => format!("SELECT mn, mx, {w} AS w FROM ({l1})"),
            WidthRule::Nice(n) => {
                let l2 = format!(
                    "SELECT mn, mx, if(mx > mn, pow(10, floor(log10((mx - mn) / {n}))), 1) AS p FROM ({l1})"
                );
                let fits = |k: u32| {
                    format!("ceil(mx / ({k} * p)) - floor(mn / ({k} * p)) <= {n}, {k} * p")
                };
                let mut branches: Vec<String> = NICE.iter().map(|k| fits(*k)).collect();
                branches.push("1000 * p".to_owned());
                format!(
                    "SELECT mn, mx, multiIf({}) AS w FROM ({l2})",
                    branches.join(", ")
                )
            }
        }
    }

    /// The one-row subquery a [`crate::builder::Relation::Ranged`] joins in:
    /// `__bmn_<c>`, `__bmx_<c>`, `__bw_<c>` over the filtered `base`.
    #[must_use]
    pub fn scalars_sql(&self, base: &str) -> String {
        format!(
            "SELECT w AS {}, mn AS {}, mx AS {} FROM ({})",
            self.w(),
            self.mn(),
            self.mx(),
            self.stats_sql(base)
        )
    }

    /// One row `(width, min, max)` over `base`: what the tile carries so the
    /// console can label the bins and know which one is last.
    #[must_use]
    pub fn read_sql(&self, base: &str) -> String {
        format!(
            "SELECT w AS width, mn AS min, mx AS max FROM ({})",
            self.stats_sql(base)
        )
    }

    /// The bin's lower edge for each row, over a relation that carries the
    /// scalars. Lower edge included, upper excluded, the maximum in the last
    /// bin (`ceil(max / w) - 1` when the range is not a point).
    #[must_use]
    pub fn bin_expr(&self) -> String {
        let (c, mn, mx, w) = (&self.column, self.mn(), self.mx(), self.w());
        format!(
            "round(least(floor(toFloat64({c}) / {w}), if({mx} > {mn}, ceil({mx} / {w}) - 1, floor({mx} / {w}))) * {w}, 10)"
        )
    }
}

/// The records predicate of a clicked bin `[lower, lower + width)` over
/// `column`, with the last bin closed: a row equal to the upper edge belongs
/// to the bin when that edge is the maximum of the same filtered rows
/// (`max_sql` is a scalar subquery over them).
#[must_use]
pub fn bin_records_predicate(
    column: &Ident,
    lower: f64,
    width: f64,
    max_sql: Option<&str>,
) -> String {
    // The same arithmetic as `bin_expr`, so a row is in the clicked bin
    // exactly when the bin expression gave it this lower edge.
    let upper = round10(lower + width);
    let lower = round10(lower);
    let open = format!(
        "(toFloat64({column}) >= {} AND toFloat64({column}) < {})",
        literal(lower),
        literal(upper)
    );
    match max_sql {
        Some(max) => format!(
            "({open} OR (toFloat64({column}) = {} AND toFloat64({column}) = ({max})))",
            literal(upper)
        ),
        None => open,
    }
}

fn round10(v: f64) -> f64 {
    (v * 1e10).round() / 1e10
}

/// `max(toFloat64(column))` over `base` as a scalar subquery.
#[must_use]
pub fn max_scalar_sql(column: &Ident, base: &str) -> String {
    format!("SELECT max(toFloat64({column})) FROM {base}")
}

// ── grid ────────────────────────────────────────────────────────────────

/// The size of a grid cell, in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GridSize {
    /// The largest of [`GRID_SIZES`] giving at least [`GRID_MIN_CELLS`] cells.
    Auto,
    /// One of [`GRID_SIZES`].
    Fixed(f64),
}

/// A grid on a point map, as written on the wire: `{ "size": "auto" | 0.01 |
/// 0.1 | 1 | 10 }`.
#[derive(Debug, Clone, PartialEq)]
pub struct GridSpec {
    /// The size.
    pub size: GridSize,
}

impl Serialize for GridSpec {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(1))?;
        match self.size {
            GridSize::Auto => map.serialize_entry("size", "auto")?,
            GridSize::Fixed(v) => map.serialize_entry("size", &v)?,
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for GridSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            size: serde_json::Value,
        }
        let raw = Raw::deserialize(deserializer)
            .map_err(|_| D::Error::custom("grid must be {\"size\": ...}"))?;
        let size = match raw.size {
            serde_json::Value::String(s) if s == "auto" => GridSize::Auto,
            serde_json::Value::String(s) => GridSize::Fixed(
                s.trim()
                    .parse::<f64>()
                    .map_err(|_| D::Error::custom("grid size must be auto or a number"))?,
            ),
            serde_json::Value::Number(n) => GridSize::Fixed(
                n.as_f64()
                    .ok_or_else(|| D::Error::custom("grid size must be a number"))?,
            ),
            _ => return Err(D::Error::custom("grid size must be auto or a number")),
        };
        Ok(Self { size })
    }
}

impl GridSpec {
    /// The size, once checked against the closed list.
    ///
    /// # Errors
    ///
    /// The sentence to show the person who chose the grid.
    pub fn checked(&self) -> Result<GridSize, String> {
        match self.size {
            GridSize::Fixed(v) if !GRID_SIZES.contains(&v) => {
                Err("a grid cell is 0.01, 0.1, 1 or 10 degrees, or automatic.".to_owned())
            }
            size => Ok(size),
        }
    }
}

/// The statement of a point map grouped by grid cell: one row per cell at the
/// cell's centre, with the aggregate of `measure`, the number of points in the
/// cell (`cell_points`) and the cell's size (`cell_size`), the largest first,
/// capped at `limit` cells.
///
/// `src` is the relation text with the chart's predicates and the coordinate
/// null checks already applied inside it ([`crate::builder::Relation::Filtered`]),
/// so a predicate on `lat` reads the raw column and not the cell. The cell
/// is computed in an inner select under other names (`__cy`, `__cx`), because
/// `f(lat) AS lat` beside a filter on `lat` is the shadowing BI-9 measured.
#[must_use]
pub fn grid_points_sql(
    src: &str,
    coords: (&Ident, &Ident),
    measure: &Ident,
    agg: Aggregate,
    size: GridSize,
    (limit, settings): (u32, &str),
) -> String {
    let (lat, lon) = coords;
    let (from, s) = match size {
        GridSize::Fixed(v) => (src.to_owned(), literal(v)),
        GridSize::Auto => {
            let counts = GRID_SIZES
                .iter()
                .take(GRID_SIZES.len() - 1)
                .map(|g| {
                    let g = literal(*g);
                    format!("uniqExact((floor({lat} / {g}), floor({lon} / {g})))")
                })
                .collect::<Vec<_>>();
            let pick = GRID_SIZES
                .iter()
                .take(GRID_SIZES.len() - 1)
                .enumerate()
                .map(|(i, g)| format!("c{i} >= {GRID_MIN_CELLS}, {}", literal(*g)))
                .chain([literal(GRID_SIZES[GRID_SIZES.len() - 1])])
                .collect::<Vec<_>>()
                .join(", ");
            let names = (0..counts.len())
                .map(|i| format!("{} AS c{i}", counts[i]))
                .collect::<Vec<_>>()
                .join(", ");
            (
                format!(
                    "{src} CROSS JOIN (SELECT multiIf({pick}) AS __gs FROM (SELECT {names} FROM {src})) AS grid"
                ),
                "__gs".to_owned(),
            )
        }
    };
    let value = if agg == Aggregate::Count {
        format!("count() AS {measure}")
    } else {
        format!("round({agg}({measure})) AS {measure}")
    };
    format!(
        "SELECT __cy AS {lat}, __cx AS {lon}, {value}, count() AS cell_points, max(__cs) AS cell_size \
         FROM (SELECT round(floor({lat} / {s}) * {s} + {s} / 2, 10) AS __cy, \
         round(floor({lon} / {s}) * {s} + {s} / 2, 10) AS __cx, {measure}, {s} AS __cs FROM {from}) \
         GROUP BY __cy, __cx ORDER BY {measure} DESC, __cy, __cx LIMIT {limit}{settings}"
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn spec(json: &str) -> BinSpec {
        serde_json::from_str(json).unwrap()
    }

    fn col(name: &str) -> Ident {
        Ident::new(name).unwrap()
    }

    #[test]
    fn the_three_modes_are_read_and_written_back() {
        assert_eq!(spec(r#"{"mode":"auto"}"#).mode(), Ok(BinMode::Auto));
        assert_eq!(
            spec(r#"{"mode":"count","count":50}"#).mode(),
            Ok(BinMode::Count(50))
        );
        assert_eq!(
            spec(r#"{"mode":"width","width":500}"#).mode(),
            Ok(BinMode::Width(500.0))
        );
        assert_eq!(
            serde_json::to_string(&spec(r#"{"mode":"width","width":500}"#)).unwrap(),
            r#"{"mode":"width","width":500}"#
        );
    }

    #[test]
    fn a_number_may_be_sent_as_a_string() {
        assert_eq!(
            spec(r#"{"mode":"count","count":"10"}"#).mode(),
            Ok(BinMode::Count(10))
        );
        assert_eq!(
            spec(r#"{"mode":"width","width":"0.5"}"#).mode(),
            Ok(BinMode::Width(0.5))
        );
    }

    #[test]
    fn a_bin_that_does_not_make_sense_is_refused_with_a_sentence() {
        for json in [
            r#"{"mode":"count","count":7}"#,
            r#"{"mode":"count"}"#,
            r#"{"mode":"width"}"#,
            r#"{"mode":"width","width":0}"#,
            r#"{"mode":"width","width":-3}"#,
            r#"{"mode":"width","width":1e30}"#,
            r#"{"mode":"width","width":"abc"}"#,
            r#"{"mode":"width","width":[1]}"#,
            r#"{"mode":"auto","width":5}"#,
            r#"{"mode":"nope"}"#,
        ] {
            assert!(spec(json).mode().is_err(), "{json}");
        }
    }

    #[test]
    fn the_range_statement_reads_the_column_once_from_the_filtered_base() {
        let r = Range::new(&col("visitors"), BinMode::Width(500.0));
        let sql = r.read_sql("serving.t");
        assert_eq!(
            sql,
            "SELECT w AS width, mn AS min, mx AS max FROM (SELECT mn, mx, 500 AS w FROM (SELECT min(toFloat64(visitors)) AS mn, max(toFloat64(visitors)) AS mx FROM serving.t))"
        );
        assert_eq!(sql.matches("serving.t").count(), 1);
    }

    #[test]
    fn an_automatic_width_tries_round_widths_until_the_bins_fit_the_count() {
        let sql = Range::new(&col("v"), BinMode::Count(10)).read_sql("serving.t");
        assert!(sql.contains("log10((mx - mn) / 10)"), "{sql}");
        assert!(sql.contains("ceil(mx / (1 * p)) - floor(mn / (1 * p)) <= 10, 1 * p"));
        assert!(sql.contains("ceil(mx / (100 * p)) - floor(mn / (100 * p)) <= 10, 100 * p"));
        assert!(sql.contains("1000 * p"));
    }

    #[test]
    fn the_bin_is_the_lower_edge_and_the_maximum_stays_in_the_last_bin() {
        let r = Range::new(&col("visitors"), BinMode::Width(500.0));
        let e = r.bin_expr();
        assert_eq!(
            e,
            "round(least(floor(toFloat64(visitors) / __bw_visitors), if(__bmx_visitors > __bmn_visitors, ceil(__bmx_visitors / __bw_visitors) - 1, floor(__bmx_visitors / __bw_visitors))) * __bw_visitors, 10)"
        );
    }

    #[test]
    fn a_width_is_printed_without_an_exponent() {
        for (w, text) in [
            (0.000_001, "0.000001"),
            (1e12, "1000000000000"),
            (2.5, "2.5"),
        ] {
            assert_eq!(literal(w), text);
        }
    }

    #[test]
    fn a_clicked_bin_is_closed_above_only_when_it_ends_at_the_maximum() {
        let c = col("visitors");
        let plain = bin_records_predicate(&c, 500.0, 500.0, None);
        assert_eq!(
            plain,
            "(toFloat64(visitors) >= 500 AND toFloat64(visitors) < 1000)"
        );
        let last = bin_records_predicate(&c, 500.0, 500.0, Some("SELECT 1"));
        assert!(last.contains("toFloat64(visitors) = 1000 AND toFloat64(visitors) = (SELECT 1)"));
    }

    #[test]
    fn a_grid_reads_a_number_or_auto_and_only_the_listed_sizes() {
        let g: GridSpec = serde_json::from_str(r#"{"size":"auto"}"#).unwrap();
        assert_eq!(g.checked(), Ok(GridSize::Auto));
        let g: GridSpec = serde_json::from_str(r#"{"size":0.1}"#).unwrap();
        assert_eq!(g.checked(), Ok(GridSize::Fixed(0.1)));
        let g: GridSpec = serde_json::from_str(r#"{"size":"1"}"#).unwrap();
        assert_eq!(g.checked(), Ok(GridSize::Fixed(1.0)));
        let g: GridSpec = serde_json::from_str(r#"{"size":0.3}"#).unwrap();
        assert!(g.checked().is_err());
        assert!(serde_json::from_str::<GridSpec>(r#"{"size":true}"#).is_err());
        assert_eq!(
            serde_json::to_string(&GridSpec {
                size: GridSize::Auto
            })
            .unwrap(),
            r#"{"size":"auto"}"#
        );
    }

    #[test]
    fn a_grid_statement_groups_by_cell_and_keeps_the_cap() {
        let sql = grid_points_sql(
            "serving.t",
            (&col("lat"), &col("lon")),
            &col("visitors"),
            Aggregate::Sum,
            GridSize::Fixed(1.0),
            (5000, ""),
        );
        assert_eq!(
            sql,
            "SELECT __cy AS lat, __cx AS lon, round(sum(visitors)) AS visitors, count() AS cell_points, max(__cs) AS cell_size FROM (SELECT round(floor(lat / 1) * 1 + 1 / 2, 10) AS __cy, round(floor(lon / 1) * 1 + 1 / 2, 10) AS __cx, visitors, 1 AS __cs FROM serving.t) GROUP BY __cy, __cx ORDER BY visitors DESC, __cy, __cx LIMIT 5000"
        );
    }

    #[test]
    fn an_automatic_grid_counts_cells_of_each_size_over_the_same_relation() {
        let sql = grid_points_sql(
            "serving.t",
            (&col("lat"), &col("lon")),
            &col("v"),
            Aggregate::Count,
            GridSize::Auto,
            (10, ""),
        );
        assert!(sql.contains("multiIf(c0 >= 50, 10, c1 >= 50, 1, c2 >= 50, 0.1, 0.01) AS __gs"));
        assert_eq!(sql.matches("FROM serving.t").count(), 2, "{sql}");
        assert!(sql.contains("count() AS v"));
    }
}
