//! Raw tables, pivot tables and KPI comparisons (`BI-16` part A).
//!
//! Everything here is either a validated shape of a chart definition or a
//! SQL statement built from one. The statements are assembled only from
//! [`Ident`]s that were checked against the relation's real columns, closed
//! enums ([`Aggregate`], [`Grain`]), numbers printed by Rust, and the
//! predicates [`crate::builder::filter_predicates`] already made through
//! [`lakehouse_core::ident::SqlLiteral`]; no caller text is ever formatted in.
//!
//! # Why the shapes are what they are
//!
//! - **Raw table** (`tableMode: "rows"`): the rows themselves, 50 a page, in
//!   an order that makes `OFFSET` pages stable (the sort, then every selected
//!   column). The same statement shape serves the dashboard's first page and
//!   the records route's later pages, so a filter, a role rewrite and a SQL
//!   source's row cap mean the same on both.
//! - **Pivot**: one `GROUP BY GROUPING SETS` statement returns the body cells
//!   and every total as long-format rows. Totals come from the engine over
//!   the underlying rows, never from adding cells up (an average of averages
//!   would be wrong). A rolled-up key is marked by `grouping()` flags
//!   (`__g0`, `__g1`, ...), never by an empty value: on `ClickHouse` 26.8 a
//!   rolled-up `Nullable` key prints as `NULL` and a rolled-up `String` or
//!   `Date` key as its default (`''`, `1970-01-01`), which a real group can
//!   hold too (measured, `docs/superpowers/plans/2026-10-11-bi-16a-tables-kpi.md`
//!   section 6).
//! - **KPI comparison**: the latest periods that have data, bucketed by the
//!   report time zone of `BI-9` ([`crate::grain::TimeContext`]); the console
//!   reads the last as "now" and the one before it as "previous".

use std::collections::{BTreeMap, HashMap, HashSet};

use lakehouse_core::ident::Ident;
use serde::{Deserialize, Serialize};

use crate::builder::{Relation, RelationColumns};
use crate::filters::ColumnKind;
use crate::grain::{Grain, TimeContext};
use crate::specs::{Aggregate, ChartKind};

/// Rows a raw table shows per page, and the most the records route returns.
pub const ROWS_PAGE_SIZE: u32 = 50;
/// Most columns a raw table lists.
pub const MAX_TABLE_COLUMNS: usize = 30;
/// Most body cells a pivot returns (`BI-16` spec, owner to confirm).
pub const PIVOT_MAX_CELLS: u32 = 10_000;
/// Rows a statement over a SQL source may return
/// ([`crate::builder::SQL_SOURCE_SETTINGS`] caps it at 2 000).
const SQL_SOURCE_MAX_ROWS: u32 = 2000;
/// Periods a KPI trend line shows.
pub const TREND_PERIODS: u32 = 12;

const COLUMN_FORMATS: [&str; 7] = [
    "auto", "number", "percent", "currency", "date", "link", "image",
];

/// One column's display settings. The server stores and returns them; the
/// console formats. Unknown keys are refused at save.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColumnSetting {
    /// Header text instead of the column's name.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub label: Option<String>,
    /// One of `auto`, `number`, `percent`, `currency`, `date`, `link`, `image`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub format: Option<String>,
    /// Digits after the point, 0 to 6.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub decimals: Option<u8>,
    /// Column width in pixels, 60 to 800.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub width: Option<u32>,
    /// Wrap long text instead of cutting it.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub wrap: Option<bool>,
    /// Leave the column out of the table.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub hidden: Option<bool>,
}

impl ColumnSetting {
    /// Check one column's settings.
    ///
    /// # Errors
    ///
    /// The message to show the person who saved them.
    pub fn validate(&self, column: &str) -> Result<(), String> {
        if let Some(label) = &self.label
            && (label.trim().is_empty()
                || label.chars().count() > 80
                || label.chars().any(char::is_control))
        {
            return Err(format!(
                "the label of '{column}' must be 1 to 80 characters."
            ));
        }
        if let Some(format) = &self.format
            && !COLUMN_FORMATS.contains(&format.as_str())
        {
            return Err(format!(
                "invalid format '{format}' for '{column}': use one of {}.",
                COLUMN_FORMATS.join(", ")
            ));
        }
        if self.decimals.is_some_and(|d| d > 6) {
            return Err(format!("decimals of '{column}' must be 0 to 6."));
        }
        if self.width.is_some_and(|w| !(60..=800).contains(&w)) {
            return Err(format!("the width of '{column}' must be 60 to 800 pixels."));
        }
        Ok(())
    }
}

/// One value of a pivot: an aggregate of a column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PivotValue {
    /// The column that is aggregated.
    pub column: String,
    /// `sum`, `avg`, `max`, `min` or `count`.
    pub aggregate: String,
}

/// What a KPI is compared with. A struct and not an enum so that an unknown
/// key is refused (`deny_unknown_fields` does not combine with an internally
/// tagged enum) and a wrong combination gets a plain message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Compare {
    /// `previous` or `goal`.
    pub kind: String,
    /// `previous`: the date or timestamp column the periods are cut from.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub date_column: Option<String>,
    /// `previous`: `day`, `week`, `month`, `quarter` or `year`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub period: Option<String>,
    /// `goal`: the number the KPI is measured against.
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        serialize_with = "crate::store::serialize_js_number"
    )]
    pub value: Option<f64>,
}

/// The fields `BI-16` part A adds to a chart definition. Flattened into
/// [`crate::store::ChartInput`] so the wire shape stays flat and a chart saved
/// before they existed reads back with every one absent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TableFields {
    /// `table`: absent or `grouped` is the grouped summary; `rows` the raw rows.
    #[serde(rename = "tableMode", skip_serializing_if = "Option::is_none", default)]
    pub table_mode: Option<String>,
    /// `table` in rows mode: the columns shown, in order. `pivot`: the column
    /// fields (0 to 2).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub columns: Option<Vec<String>>,
    /// `pivot`: the row fields (1 to 3).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub rows: Option<Vec<String>>,
    /// `pivot`: the values (1 to 5).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub values: Option<Vec<PivotValue>>,
    /// `pivot`: `none`, `grand` or `all`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub totals: Option<String>,
    /// `table` in rows mode: the column the saved order sorts by.
    #[serde(
        rename = "sortColumn",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub sort_column: Option<String>,
    /// `asc` (default) or `desc`.
    #[serde(rename = "sortDir", skip_serializing_if = "Option::is_none", default)]
    pub sort_dir: Option<String>,
    /// Per-column display settings, by column name.
    #[serde(
        rename = "columnSettings",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub column_settings: Option<BTreeMap<String, ColumnSetting>>,
    /// `kpi`: what it is compared with.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub compare: Option<Compare>,
    /// `kpi`: `up` (default) or `down` is the good direction.
    #[serde(
        rename = "goodDirection",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub good_direction: Option<String>,
}

impl TableFields {
    /// Whether the table is in raw-rows mode.
    #[must_use]
    pub fn is_rows_mode(&self, kind: ChartKind) -> bool {
        kind == ChartKind::Table && self.table_mode.as_deref() == Some("rows")
    }

    /// Whether the KPI compares with its previous period (its SQL depends on
    /// the deployment's time settings, so it is rebuilt at read time).
    #[must_use]
    pub fn compares_to_previous(&self, kind: ChartKind) -> bool {
        kind == ChartKind::Kpi && self.compare.as_ref().is_some_and(|c| c.kind == "previous")
    }
}

/// The column set a raw table lists and how it is ordered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowsPlan {
    /// The columns shown, in order.
    pub columns: Vec<Ident>,
    /// The saved sort: column and whether it is descending.
    pub sort: Option<(Ident, bool)>,
}

fn ident_of(name: &str, what: &str) -> Result<Ident, String> {
    Ident::new(name).map_err(|_| format!("invalid {what} column '{name}'."))
}

fn existing<S: std::hash::BuildHasher>(
    name: &str,
    what: &str,
    cols: &HashSet<String, S>,
) -> Result<Ident, String> {
    let ident = ident_of(name, what)?;
    if cols.contains(name) {
        Ok(ident)
    } else {
        Err(format!("invalid or missing {what} column '{name}'."))
    }
}

fn sort_of<S: std::hash::BuildHasher>(
    column: Option<&str>,
    dir: Option<&str>,
    cols: &HashSet<String, S>,
) -> Result<Option<(Ident, bool)>, String> {
    let desc = match dir {
        None | Some("asc") => false,
        Some("desc") => true,
        Some(other) => {
            return Err(format!(
                "invalid sort direction '{other}': use asc or desc."
            ));
        }
    };
    match column {
        None if dir.is_some() => Err("a sort direction needs a sort column.".to_owned()),
        None => Ok(None),
        Some(c) => Ok(Some((existing(c, "sort", cols)?, desc))),
    }
}

/// Refuse a field the chart's kind does not take, so it is never stored and
/// ignored (the way `lat`/`lon` are refused on every other kind).
fn only_for(fields: &TableFields, kind: ChartKind) -> Result<(), String> {
    let wrong = |field: &str, kinds: &str| format!("{field} is only for {kinds}.");
    let rows_table = fields.is_rows_mode(kind);
    let pivot = kind == ChartKind::Pivot;
    if fields.table_mode.is_some() && kind != ChartKind::Table {
        return Err(wrong("tableMode", "table"));
    }
    if let Some(mode) = &fields.table_mode
        && mode != "grouped"
        && mode != "rows"
    {
        return Err(format!("invalid tableMode '{mode}': use grouped or rows."));
    }
    if fields.columns.is_some() && !rows_table && !pivot {
        return Err(wrong("columns", "a table in rows mode and pivot"));
    }
    if (fields.sort_column.is_some() || fields.sort_dir.is_some()) && !rows_table {
        return Err(wrong("sortColumn and sortDir", "a table in rows mode"));
    }
    if (fields.rows.is_some() || fields.values.is_some() || fields.totals.is_some()) && !pivot {
        return Err(wrong("rows, values and totals", "pivot"));
    }
    if (fields.compare.is_some() || fields.good_direction.is_some()) && kind != ChartKind::Kpi {
        return Err(wrong("compare and goodDirection", "kpi"));
    }
    if fields.column_settings.is_some() && !pivot && kind != ChartKind::Table {
        return Err(wrong("columnSettings", "table and pivot"));
    }
    Ok(())
}

/// Check the settings against the relation: every key is a column of it, and
/// each setting is valid.
///
/// # Errors
///
/// The message to show the person who saved the chart.
pub fn check_settings<S: std::hash::BuildHasher>(
    fields: &TableFields,
    cols: &HashSet<String, S>,
) -> Result<(), String> {
    let Some(settings) = &fields.column_settings else {
        return Ok(());
    };
    for (column, setting) in settings {
        if !cols.contains(column) {
            return Err(format!(
                "column settings name an unknown column '{column}'."
            ));
        }
        setting.validate(column)?;
    }
    Ok(())
}

/// The shape checks that need no column list: which fields the kind takes.
///
/// # Errors
///
/// The message to show the person who saved the chart.
pub fn check_kind_fields(fields: &TableFields, kind: ChartKind) -> Result<(), String> {
    only_for(fields, kind)
}

/// Check a raw table's definition against the relation.
///
/// # Errors
///
/// The message to show the person who saved the chart.
pub fn plan_rows<S: std::hash::BuildHasher>(
    fields: &TableFields,
    cols: &HashSet<String, S>,
) -> Result<RowsPlan, String> {
    let names = fields.columns.as_deref().unwrap_or_default();
    if names.is_empty() || names.len() > MAX_TABLE_COLUMNS {
        return Err(format!(
            "a table in rows mode lists 1 to {MAX_TABLE_COLUMNS} columns."
        ));
    }
    let mut seen = HashSet::new();
    let mut columns = Vec::with_capacity(names.len());
    for name in names {
        if !seen.insert(name.as_str()) {
            return Err(format!("column '{name}' is listed twice."));
        }
        columns.push(existing(name, "table", cols)?);
    }
    let sort = sort_of(
        fields.sort_column.as_deref(),
        fields.sort_dir.as_deref(),
        cols,
    )?;
    check_settings(fields, cols)?;
    Ok(RowsPlan { columns, sort })
}

/// The statements behind one page of a raw table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowsSql {
    /// One page of rows.
    pub rows: String,
    /// The number of rows the pages walk through.
    pub count: String,
}

/// SQL for one page of a raw table over `from` with `predicates`, and its
/// total. `ORDER BY` is the sort, then every selected column, so two pages
/// never repeat or skip a row (a bare `LIMIT .. OFFSET` has no stable order).
#[must_use]
pub fn rows_sql(
    from: &Relation,
    plan: &RowsPlan,
    predicates: &[String],
    limit: u32,
    offset: u64,
) -> RowsSql {
    let select = plan
        .columns
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let mut order: Vec<String> = Vec::new();
    if let Some((column, desc)) = &plan.sort {
        order.push(format!("{column} {}", if *desc { "DESC" } else { "ASC" }));
    }
    order.extend(plan.columns.iter().map(ToString::to_string));
    let where_sql = if predicates.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", predicates.join(" AND "))
    };
    let from_sql = from.render();
    let settings = from.settings();
    RowsSql {
        rows: format!(
            "SELECT {select} FROM {from_sql}{where_sql} ORDER BY {} LIMIT {limit} OFFSET {offset}{settings}",
            order.join(", ")
        ),
        count: format!("SELECT count() AS n FROM {from_sql}{where_sql}{settings}"),
    }
}

/// How much of a pivot is totalled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Totals {
    /// Body cells only.
    None,
    /// Plus the grand totals: every row total, every column total, and the
    /// corner.
    Grand,
    /// Plus a subtotal for every leading prefix of the row fields and of the
    /// column fields.
    All,
}

/// A pivot definition once it is checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PivotPlan {
    /// Row fields, outermost first.
    pub rows: Vec<Ident>,
    /// Column fields, outermost first.
    pub columns: Vec<Ident>,
    /// The aggregated values.
    pub values: Vec<(Ident, Aggregate)>,
    /// Totals asked for.
    pub totals: Totals,
}

impl PivotPlan {
    /// Every key field, rows first: the order of the `__g` flags.
    fn fields(&self) -> impl Iterator<Item = &Ident> {
        self.rows.iter().chain(self.columns.iter())
    }

    /// The first row or column field of date or timestamp kind, the one a
    /// `BI-9` grain applies to. A pivot carries one grain, so which field takes
    /// it is fixed here and not chosen per request.
    #[must_use]
    pub fn grain_field<H: std::hash::BuildHasher>(
        &self,
        cols: &RelationColumns<H>,
    ) -> Option<(Ident, ColumnKind)> {
        first_date_field(self.fields().map(Ident::as_str), cols)
    }
}

/// The first of `names` that is a date or timestamp column, with its kind.
fn first_date_field<'a, H: std::hash::BuildHasher>(
    names: impl Iterator<Item = &'a str>,
    cols: &RelationColumns<H>,
) -> Option<(Ident, ColumnKind)> {
    names.into_iter().find_map(|name| {
        let kind = cols
            .get(name)
            .copied()
            .filter(|k| matches!(k, ColumnKind::Date | ColumnKind::DateTime))?;
        Some((Ident::new(name).ok()?, kind))
    })
}

/// [`PivotPlan::grain_field`] for a stored definition, whose identifiers are
/// not re-validated against the relation here (the builder does that when it
/// builds the statement).
#[must_use]
pub fn pivot_grain_field<H: std::hash::BuildHasher>(
    fields: &TableFields,
    cols: &RelationColumns<H>,
) -> Option<(Ident, ColumnKind)> {
    let names = fields
        .rows
        .iter()
        .flatten()
        .chain(fields.columns.iter().flatten());
    first_date_field(names.map(String::as_str), cols)
}

/// Check a pivot's definition against the relation.
///
/// # Errors
///
/// The message to show the person who saved the chart.
pub fn plan_pivot<S: std::hash::BuildHasher>(
    fields: &TableFields,
    cols: &HashSet<String, S>,
) -> Result<PivotPlan, String> {
    let rows = fields.rows.as_deref().unwrap_or_default();
    let columns = fields.columns.as_deref().unwrap_or_default();
    let values = fields.values.as_deref().unwrap_or_default();
    if rows.is_empty() || rows.len() > 3 {
        return Err("a pivot needs 1 to 3 row fields.".to_owned());
    }
    if columns.len() > 2 {
        return Err("a pivot takes at most 2 column fields.".to_owned());
    }
    if values.is_empty() || values.len() > 5 {
        return Err("a pivot needs 1 to 5 values.".to_owned());
    }
    let mut seen = HashSet::new();
    let mut plan = PivotPlan {
        rows: Vec::new(),
        columns: Vec::new(),
        values: Vec::new(),
        totals: match fields.totals.as_deref() {
            None | Some("none") => Totals::None,
            Some("grand") => Totals::Grand,
            Some("all") => Totals::All,
            Some(other) => {
                return Err(format!("invalid totals '{other}': use none, grand or all."));
            }
        },
    };
    for (names, target) in [(rows, &mut plan.rows), (columns, &mut plan.columns)] {
        for name in names {
            if !seen.insert(name.as_str()) {
                return Err(format!("field '{name}' is used twice."));
            }
            target.push(existing(name, "pivot", cols)?);
        }
    }
    for v in values {
        let agg = v.aggregate.to_lowercase();
        if !matches!(agg.as_str(), "sum" | "avg" | "max" | "min" | "count") {
            return Err(format!("invalid aggregate: {}", v.aggregate));
        }
        plan.values.push((
            existing(&v.column, "value", cols)?,
            Aggregate::from_str_lossy(&agg),
        ));
    }
    check_settings(fields, cols)?;
    Ok(plan)
}

/// The most long-format rows a pivot statement keeps: the cell cap divided by
/// the values per row, and below the row cap of a SQL source, so
/// `ClickHouse` never cuts the result by itself (the console could not tell
/// that from a result that fits). The statement asks for one more, so the
/// caller can tell a cut from a fit ([`pivot_trim`]).
#[must_use]
pub fn pivot_cap(over_sql_source: bool, values: usize) -> u32 {
    let per_row = u32::try_from(values.max(1)).unwrap_or(1);
    let cap = PIVOT_MAX_CELLS / per_row;
    if over_sql_source {
        cap.min(SQL_SOURCE_MAX_ROWS - 1)
    } else {
        cap
    }
}

/// [`pivot_cap`] for the relation a statement reads.
#[must_use]
pub fn pivot_row_cap(from: &Relation, values: usize) -> u32 {
    fn is_sql(from: &Relation) -> bool {
        match from {
            Relation::Mart(_) => false,
            Relation::Sql(_) => true,
            Relation::Filtered { base, .. }
            | Relation::Bucketed { base, .. }
            | Relation::Calculated { base, .. } => is_sql(base),
        }
    }
    pivot_cap(is_sql(from), values)
}

/// Keep the first `cap` rows of a pivot result and say whether rows were cut.
pub fn pivot_trim<T>(rows: &mut Vec<T>, cap: u32) -> bool {
    let cap = cap as usize;
    if rows.len() > cap {
        rows.truncate(cap);
        true
    } else {
        false
    }
}

/// `GROUPING SETS` of a pivot: every combination of a leading prefix of the
/// row fields and a leading prefix of the column fields that `totals` asks
/// for (`none`: the full fields only; `grand`: nothing or everything of each
/// side; `all`: every prefix).
fn grouping_sets(plan: &PivotPlan) -> Vec<Vec<String>> {
    let prefixes = |n: usize| -> Vec<usize> {
        match plan.totals {
            Totals::None => vec![n],
            Totals::Grand if n == 0 => vec![0],
            Totals::Grand => vec![n, 0],
            Totals::All => (0..=n).rev().collect(),
        }
    };
    let mut sets = Vec::new();
    for r in prefixes(plan.rows.len()) {
        for c in prefixes(plan.columns.len()) {
            sets.push(
                plan.rows
                    .iter()
                    .take(r)
                    .chain(plan.columns.iter().take(c))
                    .map(ToString::to_string)
                    .collect(),
            );
        }
    }
    sets
}

/// Pivot SQL over `from`: the key fields, one `__v<i>` per value, and one
/// `__g<j>` flag per key field that is `1` when that field is rolled up in
/// the row (a total), `0` when the row is a group of it.
///
/// Rolled-up rows come first, then the body, each in key order, so the cell
/// cut lands on body cells and never drops a total. With a `grain`, the field
/// it applies to is replaced by its bucket in an inner relation, and the
/// `predicates` are applied to the raw rows under it
/// ([`Relation::Bucketed`] over [`Relation::Filtered`]); `None` when the grain
/// does not fit the field.
#[must_use]
pub fn pivot_sql(
    from: &Relation,
    plan: &PivotPlan,
    predicates: Vec<String>,
    grain: Option<(Grain, &Ident, ColumnKind)>,
    time: &TimeContext,
) -> Option<String> {
    let (relation, where_sql) = match grain {
        Some((g, field, kind)) => {
            let expr = g.bucket_expr(field, kind, time)?;
            let base = if predicates.is_empty() {
                from.clone()
            } else {
                Relation::Filtered {
                    base: Box::new(from.clone()),
                    predicates,
                }
            };
            let mut carried: Vec<String> = Vec::new();
            for name in plan
                .fields()
                .chain(plan.values.iter().map(|(c, _)| c))
                .map(ToString::to_string)
            {
                if name != field.as_str() && !carried.contains(&name) {
                    carried.push(name);
                }
            }
            (
                Relation::Bucketed {
                    base: Box::new(base),
                    dimension: field.to_string(),
                    expr,
                    carried,
                },
                String::new(),
            )
        }
        None => (
            from.clone(),
            if predicates.is_empty() {
                String::new()
            } else {
                format!(" WHERE {}", predicates.join(" AND "))
            },
        ),
    };
    let keys: Vec<String> = plan.fields().map(ToString::to_string).collect();
    let values = plan
        .values
        .iter()
        .enumerate()
        .map(|(i, (column, agg))| {
            if *agg == Aggregate::Count {
                format!("count() AS __v{i}")
            } else {
                format!("{agg}({column}) AS __v{i}")
            }
        })
        .collect::<Vec<_>>();
    let flags = keys
        .iter()
        .enumerate()
        .map(|(j, k)| format!("grouping({k}) AS __g{j}"))
        .collect::<Vec<_>>();
    let sets = grouping_sets(plan)
        .iter()
        .map(|s| format!("({})", s.join(", ")))
        .collect::<Vec<_>>()
        .join(", ");
    let all_keys = keys.join(", ");
    let select = keys
        .iter()
        .cloned()
        .chain(values)
        .chain(flags)
        .collect::<Vec<_>>()
        .join(", ");
    let cap = pivot_row_cap(&relation, plan.values.len()) + 1;
    Some(format!(
        "SELECT {select} FROM {}{where_sql} GROUP BY GROUPING SETS ({sets}) \
         ORDER BY grouping({all_keys}) DESC, {all_keys} LIMIT {cap}{}",
        relation.render(),
        relation.settings(),
    ))
}

/// A KPI comparison once it is checked.
#[derive(Debug, Clone, PartialEq)]
pub enum ComparePlan {
    /// The latest period that has data against the one before it.
    Previous {
        /// The date or timestamp column the periods are cut from.
        column: Ident,
        /// Its kind.
        kind: ColumnKind,
        /// The period.
        period: Grain,
    },
    /// The KPI against a number.
    Goal(f64),
}

/// Check a KPI's comparison against the relation.
///
/// # Errors
///
/// The message to show the person who saved the chart.
pub fn plan_compare<H: std::hash::BuildHasher, S: std::hash::BuildHasher>(
    fields: &TableFields,
    cols: &HashSet<String, S>,
    kinds: &HashMap<String, ColumnKind, H>,
) -> Result<Option<ComparePlan>, String> {
    if let Some(dir) = &fields.good_direction
        && dir != "up"
        && dir != "down"
    {
        return Err(format!("invalid goodDirection '{dir}': use up or down."));
    }
    let Some(c) = &fields.compare else {
        return Ok(None);
    };
    match c.kind.as_str() {
        "previous" => {
            if c.value.is_some() {
                return Err("a comparison with the previous period takes no value.".to_owned());
            }
            let (Some(column), Some(period)) = (c.date_column.as_deref(), c.period.as_deref())
            else {
                return Err(
                    "a comparison with the previous period needs a date column and a period."
                        .to_owned(),
                );
            };
            let period = Grain::parse(period)
                .filter(|g| {
                    matches!(
                        g,
                        Grain::Day | Grain::Week | Grain::Month | Grain::Quarter | Grain::Year
                    )
                })
                .ok_or_else(|| {
                    "invalid period: use day, week, month, quarter or year.".to_owned()
                })?;
            let ident = existing(column, "date", cols)?;
            let kind = kinds.get(column).copied().unwrap_or(ColumnKind::Text);
            if !matches!(kind, ColumnKind::Date | ColumnKind::DateTime) {
                return Err("a comparison needs a date or timestamp column.".to_owned());
            }
            Ok(Some(ComparePlan::Previous {
                column: ident,
                kind,
                period,
            }))
        }
        "goal" => {
            if c.date_column.is_some() || c.period.is_some() {
                return Err("a comparison with a goal takes a value only.".to_owned());
            }
            match c.value {
                Some(v) if v.is_finite() => Ok(Some(ComparePlan::Goal(v))),
                _ => Err("a goal must be a finite number.".to_owned()),
            }
        }
        other => Err(format!(
            "invalid comparison '{other}': use previous or goal."
        )),
    }
}

/// The statement behind a KPI that compares with its previous period: the
/// latest [`TREND_PERIODS`] periods that have data, oldest first, as the
/// period start (named like the date column) and the aggregate `v`. A row
/// with no date is left out. The periods are cut in the report zone
/// ([`Grain::bucket_expr`]), and `predicates` filter the raw rows under the
/// bucketing, so a filter on the date column itself reads the raw column.
#[must_use]
pub fn kpi_trend_sql(
    from: &Relation,
    measure: &Ident,
    agg: Aggregate,
    predicates: Vec<String>,
    compare: &ComparePlan,
    time: &TimeContext,
) -> Option<String> {
    let ComparePlan::Previous {
        column,
        kind,
        period,
    } = compare
    else {
        return None;
    };
    let expr = period.bucket_expr(column, *kind, time)?;
    let base = if predicates.is_empty() {
        from.clone()
    } else {
        Relation::Filtered {
            base: Box::new(from.clone()),
            predicates,
        }
    };
    let carried = if measure.as_str() == column.as_str() {
        Vec::new()
    } else {
        vec![measure.to_string()]
    };
    let bucketed = Relation::Bucketed {
        base: Box::new(base),
        dimension: column.to_string(),
        expr,
        carried,
    };
    let value = if agg == Aggregate::Count {
        "count()".to_owned()
    } else {
        format!("round({agg}({measure}))")
    };
    Some(format!(
        "SELECT * FROM (SELECT {column}, {value} AS v FROM {} WHERE {column} IS NOT NULL \
         GROUP BY {column} ORDER BY {column} DESC LIMIT {TREND_PERIODS}) ORDER BY {column}{}",
        bucketed.render(),
        bucketed.settings(),
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn cols(names: &[&str]) -> HashSet<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    fn mart() -> Relation {
        Relation::Mart(Ident::new("mart_demo").unwrap())
    }

    fn fields(json: &str) -> TableFields {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn a_rows_table_lists_one_to_thirty_distinct_existing_columns() {
        let c = cols(&["a", "b", "c"]);
        let ok = plan_rows(
            &fields(r#"{"columns":["b","a"],"sortColumn":"c","sortDir":"desc"}"#),
            &c,
        )
        .unwrap();
        assert_eq!(ok.columns.len(), 2);
        assert_eq!(
            ok.sort.as_ref().map(|(i, d)| (i.as_str(), *d)),
            Some(("c", true))
        );
        for bad in [
            r#"{"columns":[]}"#,
            r#"{"columns":["a","a"]}"#,
            r#"{"columns":["zzz"]}"#,
            r#"{"columns":["a"],"sortColumn":"zzz"}"#,
            r#"{"columns":["a"],"sortDir":"desc"}"#,
            r#"{"columns":["a"],"sortColumn":"a","sortDir":"up"}"#,
        ] {
            assert!(plan_rows(&fields(bad), &c).is_err(), "{bad}");
        }
        let many: Vec<String> = (0..31).map(|i| format!("c{i}")).collect();
        let c31: HashSet<String> = many.iter().cloned().collect();
        let f = TableFields {
            columns: Some(many),
            ..TableFields::default()
        };
        assert!(plan_rows(&f, &c31).is_err());
    }

    #[test]
    fn a_rows_page_orders_by_the_sort_then_every_column_and_counts_the_same_rows() {
        let plan = plan_rows(
            &fields(r#"{"columns":["a","b"],"sortColumn":"b","sortDir":"desc"}"#),
            &cols(&["a", "b"]),
        )
        .unwrap();
        let sql = rows_sql(&mart(), &plan, &["a > 1".to_owned()], 50, 100);
        assert_eq!(
            sql.rows,
            "SELECT a, b FROM serving.mart_demo WHERE a > 1 ORDER BY b DESC, a, b LIMIT 50 OFFSET 100"
        );
        assert_eq!(
            sql.count,
            "SELECT count() AS n FROM serving.mart_demo WHERE a > 1"
        );
    }

    #[test]
    fn a_rows_page_over_a_sql_source_carries_the_cap_on_both_statements() {
        let plan = plan_rows(&fields(r#"{"columns":["a"]}"#), &cols(&["a"])).unwrap();
        let sql = rows_sql(
            &Relation::Sql("SELECT 1 AS a".to_owned()),
            &plan,
            &[],
            50,
            0,
        );
        assert!(
            sql.rows.ends_with(crate::builder::SQL_SOURCE_SETTINGS),
            "{}",
            sql.rows
        );
        assert!(
            sql.count.ends_with(crate::builder::SQL_SOURCE_SETTINGS),
            "{}",
            sql.count
        );
    }

    #[test]
    fn column_settings_are_checked_and_unknown_keys_are_refused() {
        assert!(serde_json::from_str::<ColumnSetting>(r#"{"label":"x","color":"red"}"#).is_err());
        let c = cols(&["a"]);
        plan_rows(
            &fields(
                r#"{"columns":["a"],"columnSettings":{"a":{"label":"Visitors","format":"currency","decimals":2,"width":120,"wrap":true,"hidden":false}}}"#,
            ),
            &c,
        )
        .unwrap();
        for bad in [
            r#"{"columns":["a"],"columnSettings":{"a":{"format":"emoji"}}}"#,
            r#"{"columns":["a"],"columnSettings":{"a":{"decimals":7}}}"#,
            r#"{"columns":["a"],"columnSettings":{"a":{"width":59}}}"#,
            r#"{"columns":["a"],"columnSettings":{"a":{"width":801}}}"#,
            r#"{"columns":["a"],"columnSettings":{"a":{"label":""}}}"#,
            r#"{"columns":["a"],"columnSettings":{"zzz":{"label":"x"}}}"#,
        ] {
            assert!(plan_rows(&fields(bad), &c).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_field_the_kind_does_not_take_is_refused_not_ignored() {
        let rows = fields(r#"{"tableMode":"rows","columns":["a"]}"#);
        assert!(check_kind_fields(&rows, ChartKind::Table).is_ok());
        assert!(check_kind_fields(&rows, ChartKind::Bar).is_err());
        let grouped_with_columns = fields(r#"{"columns":["a"]}"#);
        assert!(check_kind_fields(&grouped_with_columns, ChartKind::Table).is_err());
        let pivot =
            fields(r#"{"rows":["a"],"values":[{"column":"b","aggregate":"sum"}],"totals":"all"}"#);
        assert!(check_kind_fields(&pivot, ChartKind::Pivot).is_ok());
        assert!(check_kind_fields(&pivot, ChartKind::Table).is_err());
        let compare = fields(r#"{"compare":{"kind":"goal","value":5},"goodDirection":"down"}"#);
        assert!(check_kind_fields(&compare, ChartKind::Kpi).is_ok());
        assert!(check_kind_fields(&compare, ChartKind::Gauge).is_err());
        assert!(check_kind_fields(&fields(r#"{"tableMode":"cube"}"#), ChartKind::Table).is_err());
    }

    fn pivot_plan(json: &str) -> PivotPlan {
        plan_pivot(&fields(json), &cols(&["p", "k", "m", "v", "w"])).unwrap()
    }

    #[test]
    fn a_pivot_needs_rows_values_and_distinct_fields_within_the_limits() {
        let c = cols(&["p", "k", "m", "v"]);
        let ok = r#"{"rows":["p"],"values":[{"column":"v","aggregate":"sum"}]}"#;
        assert!(plan_pivot(&fields(ok), &c).is_ok());
        for bad in [
            r#"{"rows":[],"values":[{"column":"v","aggregate":"sum"}]}"#,
            r#"{"rows":["p","k","m","v"],"values":[{"column":"v","aggregate":"sum"}]}"#,
            r#"{"rows":["p"],"columns":["k","m","v"],"values":[{"column":"v","aggregate":"sum"}]}"#,
            r#"{"rows":["p"],"columns":["p"],"values":[{"column":"v","aggregate":"sum"}]}"#,
            r#"{"rows":["p"],"values":[]}"#,
            r#"{"rows":["p"],"values":[{"column":"v","aggregate":"sum"},{"column":"v","aggregate":"avg"},{"column":"v","aggregate":"min"},{"column":"v","aggregate":"max"},{"column":"v","aggregate":"count"},{"column":"v","aggregate":"sum"}]}"#,
            r#"{"rows":["p"],"values":[{"column":"v","aggregate":"median"}]}"#,
            r#"{"rows":["p"],"values":[{"column":"zzz","aggregate":"sum"}]}"#,
            r#"{"rows":["p"],"values":[{"column":"v","aggregate":"sum"}],"totals":"some"}"#,
        ] {
            assert!(plan_pivot(&fields(bad), &c).is_err(), "{bad}");
        }
    }

    #[test]
    fn totals_all_asks_for_every_prefix_of_both_sides_and_grand_only_the_ends() {
        let all = pivot_plan(
            r#"{"rows":["p","k"],"columns":["m"],"values":[{"column":"v","aggregate":"sum"}],"totals":"all"}"#,
        );
        assert_eq!(
            grouping_sets(&all),
            vec![
                vec!["p", "k", "m"],
                vec!["p", "k"],
                vec!["p", "m"],
                vec!["p"],
                vec!["m"],
                Vec::<&str>::new(),
            ]
            .into_iter()
            .map(|s| s.into_iter().map(str::to_owned).collect::<Vec<_>>())
            .collect::<Vec<_>>()
        );
        let grand = pivot_plan(
            r#"{"rows":["p","k"],"columns":["m"],"values":[{"column":"v","aggregate":"sum"}],"totals":"grand"}"#,
        );
        assert_eq!(grouping_sets(&grand).len(), 4);
        let none = pivot_plan(r#"{"rows":["p"],"values":[{"column":"v","aggregate":"sum"}]}"#);
        assert_eq!(grouping_sets(&none), vec![vec!["p".to_owned()]]);
        let rows_only = pivot_plan(
            r#"{"rows":["p"],"values":[{"column":"v","aggregate":"sum"}],"totals":"grand"}"#,
        );
        assert_eq!(
            grouping_sets(&rows_only),
            vec![vec!["p".to_owned()], Vec::<String>::new()]
        );
    }

    #[test]
    fn a_pivot_statement_marks_rolled_up_keys_by_grouping_and_puts_totals_first() {
        let plan = pivot_plan(
            r#"{"rows":["p","k"],"columns":["m"],"values":[{"column":"v","aggregate":"sum"},{"column":"w","aggregate":"count"}],"totals":"grand"}"#,
        );
        let sql = pivot_sql(
            &mart(),
            &plan,
            vec!["v > 0".to_owned()],
            None,
            &TimeContext::default(),
        )
        .unwrap();
        assert_eq!(
            sql,
            "SELECT p, k, m, sum(v) AS __v0, count() AS __v1, grouping(p) AS __g0, grouping(k) AS __g1, grouping(m) AS __g2 \
             FROM serving.mart_demo WHERE v > 0 GROUP BY GROUPING SETS ((p, k, m), (p, k), (m), ()) \
             ORDER BY grouping(p, k, m) DESC, p, k, m LIMIT 5001"
        );
    }

    #[test]
    fn the_cell_cap_divides_by_the_values_and_stays_under_a_sql_sources_row_cap() {
        assert_eq!(pivot_row_cap(&mart(), 1), 10_000);
        assert_eq!(pivot_row_cap(&mart(), 5), 2_000);
        assert_eq!(
            pivot_row_cap(&Relation::Sql("SELECT 1".to_owned()), 1),
            1_999
        );
        let mut rows = vec![1, 2, 3];
        assert!(pivot_trim(&mut rows, 2));
        assert_eq!(rows, vec![1, 2]);
        assert!(!pivot_trim(&mut rows, 2));
    }

    #[test]
    fn a_pivot_with_a_grain_buckets_the_field_under_the_filters_and_carries_the_rest() {
        let plan = pivot_plan(
            r#"{"rows":["p"],"columns":["m"],"values":[{"column":"v","aggregate":"sum"}]}"#,
        );
        let m = Ident::new("m").unwrap();
        let sql = pivot_sql(
            &mart(),
            &plan,
            vec!["p = 'a'".to_owned()],
            Some((Grain::Month, &m, ColumnKind::Date)),
            &TimeContext::default(),
        )
        .unwrap();
        assert!(
            sql.contains(
                "FROM (SELECT date_trunc('month', m) AS m, p, v FROM (SELECT * FROM serving.mart_demo WHERE p = 'a') AS flt) AS bkt GROUP BY GROUPING SETS"
            ),
            "{sql}"
        );
        assert!(!sql.contains(" WHERE p = 'a' GROUP"), "{sql}");
        // An hour on a plain date has no meaning.
        assert!(
            pivot_sql(
                &mart(),
                &plan,
                Vec::new(),
                Some((Grain::Hour, &m, ColumnKind::Date)),
                &TimeContext::default()
            )
            .is_none()
        );
    }

    fn kinds() -> HashMap<String, ColumnKind> {
        HashMap::from([
            ("d".to_owned(), ColumnKind::Date),
            ("t".to_owned(), ColumnKind::DateTime),
            ("x".to_owned(), ColumnKind::Text),
            ("v".to_owned(), ColumnKind::Number),
        ])
    }

    fn compare(json: &str) -> Result<Option<ComparePlan>, String> {
        plan_compare(&fields(json), &cols(&["d", "t", "x", "v"]), &kinds())
    }

    #[test]
    fn a_comparison_is_the_previous_period_of_a_date_column_or_a_finite_goal() {
        assert!(matches!(
            compare(r#"{"compare":{"kind":"previous","dateColumn":"d","period":"month"}}"#),
            Ok(Some(ComparePlan::Previous {
                period: Grain::Month,
                kind: ColumnKind::Date,
                ..
            }))
        ));
        assert!(matches!(
            compare(r#"{"compare":{"kind":"goal","value":1200.5},"goodDirection":"down"}"#),
            Ok(Some(ComparePlan::Goal(v))) if (v - 1200.5).abs() < f64::EPSILON
        ));
        assert_eq!(compare(r"{}"), Ok(None));
        for bad in [
            r#"{"compare":{"kind":"previous","dateColumn":"x","period":"month"}}"#,
            r#"{"compare":{"kind":"previous","dateColumn":"zzz","period":"month"}}"#,
            r#"{"compare":{"kind":"previous","dateColumn":"d","period":"hour"}}"#,
            r#"{"compare":{"kind":"previous","dateColumn":"d"}}"#,
            r#"{"compare":{"kind":"previous","dateColumn":"d","period":"day","value":1}}"#,
            r#"{"compare":{"kind":"goal"}}"#,
            r#"{"compare":{"kind":"goal","value":1,"period":"day"}}"#,
            r#"{"compare":{"kind":"average"}}"#,
            r#"{"goodDirection":"sideways"}"#,
        ] {
            assert!(compare(bad).is_err(), "{bad}");
        }
        assert!(
            serde_json::from_str::<TableFields>(
                r#"{"compare":{"kind":"goal","value":1,"colour":"red"}}"#
            )
            .is_err()
        );
    }

    #[test]
    fn a_previous_period_statement_keeps_the_latest_twelve_periods_with_data_in_the_report_zone() {
        let plan = compare(r#"{"compare":{"kind":"previous","dateColumn":"t","period":"week"}}"#)
            .unwrap()
            .unwrap();
        let measure = Ident::new("v").unwrap();
        let sql = kpi_trend_sql(
            &mart(),
            &measure,
            Aggregate::Sum,
            vec!["x = 'a'".to_owned()],
            &plan,
            &TimeContext::default(),
        )
        .unwrap();
        assert!(sql.starts_with("SELECT * FROM (SELECT t, round(sum(v)) AS v FROM (SELECT date_trunc('week', toTimeZone(t, 'Asia/Jakarta')) AS t, v FROM (SELECT * FROM serving.mart_demo WHERE x = 'a') AS flt) AS bkt WHERE t IS NOT NULL GROUP BY t ORDER BY t DESC LIMIT 12) ORDER BY t"), "{sql}");
        let goal = compare(r#"{"compare":{"kind":"goal","value":3}}"#)
            .unwrap()
            .unwrap();
        assert!(
            kpi_trend_sql(
                &mart(),
                &measure,
                Aggregate::Sum,
                Vec::new(),
                &goal,
                &TimeContext::default()
            )
            .is_none()
        );
    }
}
