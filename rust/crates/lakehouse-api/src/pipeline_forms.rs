//! BLD-16 (`BLD-1` part 2) / J2 to J5: parse the stored fields of a
//! `join`, `aggregate` or `filter` step into checked values, and render
//! each into the one `SELECT` that is the step's CTE body.
//!
//! Pure: no I/O, no state, no schema. The compiler
//! ([`crate::pipeline_compile`]) calls the parsers in `validate` (so a
//! refusal happens at save, preview, run and compile time alike — D9's
//! one answer) and the renderers in `step_body`, so the body inside a
//! step's `WITH` entry and the text [`step_sql`](crate::pipeline_compile)
//! hands the console are produced by the same functions and cannot
//! differ.
//!
//! # Why each kind is shaped the way it is
//!
//! - A join lists its output columns (F1): `SELECT *` over a join in
//!   `ClickHouse` names a duplicate right-side column
//!   `<alias>.<column>`, and this module is pure — it cannot know the
//!   columns, so it refuses to guess and makes the caller list them.
//!   `__left` and `__right` are fixed aliases; D2 refuses a name
//!   starting with `__`, so no input or step can collide with them.
//! - A measure's `name` may not be a column the step names (F2): in
//!   `ClickHouse` an alias is visible in the whole query, so
//!   `sum(amount) AS amount, avg(amount) AS a` makes the second read
//!   `avg(sum(amount))` — an engine error, not an answer.
//! - A `having` condition is rendered with the measure's EXPRESSION, not
//!   its alias, for the same reason: the alias question of F2 cannot
//!   arise in a `HAVING` this module writes.
//! - An outer join's missing side keeps the engine's default (`0` or
//!   empty text), not a blank (F3): the orchestrator appends its own
//!   `SETTINGS` after the compiled statement, so this module cannot add
//!   `join_use_nulls` — true of a SQL step today, and kept the same for
//!   both kinds rather than made silently different.
//!
//! # The invariant this module keeps (plan section 3, rules 4 and 5)
//!
//! No identifier from user text reaches SQL unchecked: every column,
//! output name and measure name goes through
//! [`lakehouse_core::ident::Ident::new`] (and a measure's name through
//! D2's `valid_name`, since it becomes an output column), and is written
//! between backticks; a name that fails is refused with its path, never
//! escaped, trimmed or guessed at. No value reaches SQL unescaped: a
//! pipeline's statement is one text the orchestrator runs, so a value
//! cannot be a bind parameter here — every value goes through
//! [`lakehouse_core::ident::SqlLiteral`], or, only for J4's numbers,
//! through [`is_decimal`] and is rendered unquoted only when every
//! character is a digit, one leading `-` or one `.` between digits.
//! `format!` in this module formats only checked values and constant
//! text.
//!
//! Every refusal is a [`DefinitionError`] with a `&'static str` from the
//! plan's section-4 list and a path naming `steps[i].<field>[j]` (D7) —
//! fixed text, never the caller's own words.

use lakehouse_core::ident::{Ident, SqlLiteral};
use lakehouse_store::pipelines::{JoinColumn, JoinPair, Measure, StepCondition};

use crate::pipeline_compile::{DefinitionError, valid_name};
use crate::transform_grammar::filter_operator;

/// At most 10 `on` pairs per join (J2, planner default).
const MAX_JOIN_PAIRS: usize = 10;
/// At most 300 output columns per join (J2, planner default).
const MAX_JOIN_COLUMNS: usize = 300;
/// At most 20 group-by columns per aggregate (J2, planner default).
const MAX_GROUP_COLUMNS: usize = 20;
/// At most 50 measures per aggregate (J2, planner default).
const MAX_MEASURES: usize = 50;
/// At most 20 conditions per filter or `having` list (J2, planner default).
const MAX_CONDITIONS: usize = 20;
/// Longest condition value, in characters (J2, planner default).
const MAX_VALUE_CHARS: usize = 1_000;
/// Digits before and after the dot a `having` number may carry (J4).
const MAX_NUMBER_DIGITS: usize = 30;

// ── the fixed sentences (the plan's section 4) ─────────────────────────
const JOIN_TYPE: &str = "a join is inner, left, right or full";
const JOIN_NO_PAIR: &str = "a join needs at least one pair of columns to match on";
const JOIN_TOO_MANY_PAIRS: &str = "a join matches on at most 10 pairs of columns";
const JOIN_NO_COLUMN: &str = "a join needs at least one output column";
const JOIN_TOO_MANY_COLUMNS: &str = "a join has at most 300 output columns";
const JOIN_SIDE: &str = "an output column comes from the left or the right side";
const COLUMN_NAME: &str =
    "a column must be a plain name (letters, digits and underscores, not starting with a digit)";
const DUPLICATE_OUTPUT: &str =
    "another output column of this step has this name; give one of them another name";
const MATCH_MODE: &str = "match is all or any";
const NO_CONDITION: &str = "a filter needs at least one condition";
const TOO_MANY_CONDITIONS: &str = "a step has at most 20 conditions";
const OPERATOR: &str = "a condition's operator is one of = != < > <= >=";
const VALUE_TOO_LONG: &str = "a condition's value is at most 1,000 characters";
const AGGREGATE_EMPTY: &str = "an aggregation needs a column to group by or a measure";
const TOO_MANY_GROUP_COLUMNS: &str = "an aggregation groups by at most 20 columns";
const TOO_MANY_MEASURES: &str = "an aggregation has at most 50 measures";
const MEASURE_FUNCTION: &str = "a measure is count, countDistinct, sum, avg, min or max";
const MEASURE_NEEDS_COLUMN: &str = "this measure needs a column";
const MEASURE_NAME_IS_COLUMN: &str =
    "a measure's name cannot be a column this step reads; choose another name";
const HAVING_UNKNOWN: &str = "a condition here names a group-by column or a measure of this step";
const HAVING_NOT_A_NUMBER: &str = "a count, sum or average is compared with a number";

/// The refusal at `steps[index]` (the step itself, no field).
fn step_err(index: usize, message: &'static str) -> DefinitionError {
    DefinitionError::at(format!("steps[{index}]"), message)
}

/// The refusal at `steps[index].<field>`.
fn field_err(index: usize, field: &str, message: &'static str) -> DefinitionError {
    DefinitionError::at(format!("steps[{index}].{field}"), message)
}

/// The refusal at `steps[index].<field>[j]`.
fn row_err(index: usize, field: &str, j: usize, message: &'static str) -> DefinitionError {
    DefinitionError::at(format!("steps[{index}].{field}[{j}]"), message)
}

/// The four join types the spec allows (Metabase's four).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinKind {
    /// Only rows that match on both sides.
    Inner,
    /// Every row of the left, matched or not.
    Left,
    /// Every row of the right, matched or not.
    Right,
    /// Every row of both sides.
    Full,
}

impl JoinKind {
    /// The stored text (J2: `inner`, `left`, `right`, `full`), or `None`.
    #[must_use]
    fn parse(text: &str) -> Option<Self> {
        match text {
            "inner" => Some(Self::Inner),
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            "full" => Some(Self::Full),
            _ => None,
        }
    }

    /// The `JOIN` keyword the renderer writes.
    #[must_use]
    fn keyword(self) -> &'static str {
        match self {
            Self::Inner => "INNER JOIN",
            Self::Left => "LEFT JOIN",
            Self::Right => "RIGHT JOIN",
            Self::Full => "FULL OUTER JOIN",
        }
    }
}

/// Which side of a join an output column comes from (J2: `left` or
/// `right`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// The step's `left` input, aliased `__left`.
    Left,
    /// The step's `right` input, aliased `__right`.
    Right,
}

impl Side {
    /// The stored text, or `None`.
    #[must_use]
    fn parse(text: &str) -> Option<Self> {
        match text {
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            _ => None,
        }
    }

    /// The fixed alias this side is read through.
    #[must_use]
    fn alias(self) -> &'static str {
        match self {
            Self::Left => "__left",
            Self::Right => "__right",
        }
    }
}

/// How a list of conditions is joined: `all` (` AND `) or `any`
/// (` OR `). Empty or missing stored text means `all` (J2), so the
/// assistant may leave the field out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchMode {
    /// Every condition must hold.
    All,
    /// At least one condition must hold.
    Any,
}

impl MatchMode {
    /// The stored text (J2: `all`, `any`, empty means `all`), or `None`.
    #[must_use]
    fn parse(text: &str) -> Option<Self> {
        match text {
            "" | "all" => Some(Self::All),
            "any" => Some(Self::Any),
            _ => None,
        }
    }

    /// The joiner between this list's conditions.
    #[must_use]
    fn joiner(self) -> &'static str {
        match self {
            Self::All => " AND ",
            Self::Any => " OR ",
        }
    }
}

/// The six measure functions an aggregate offers (J2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeasureFunction {
    /// `count()` counts rows; `count(c)` counts rows where `c` is not
    /// empty.
    Count,
    /// `count(DISTINCT c)`.
    CountDistinct,
    /// `sum(c)`.
    Sum,
    /// `avg(c)`.
    Avg,
    /// `min(c)`.
    Min,
    /// `max(c)`.
    Max,
}

impl MeasureFunction {
    /// The stored text (J2: `count`, `countDistinct`, `sum`, `avg`, `min`,
    /// `max`), or `None`.
    #[must_use]
    fn parse(text: &str) -> Option<Self> {
        match text {
            "count" => Some(Self::Count),
            "countDistinct" => Some(Self::CountDistinct),
            "sum" => Some(Self::Sum),
            "avg" => Some(Self::Avg),
            "min" => Some(Self::Min),
            "max" => Some(Self::Max),
            _ => None,
        }
    }

    /// Whether a `having` value compared with this function's measure must
    /// be a number (J4): a count, a distinct count, a sum or an average is
    /// a number by construction; a min or max of a text column is text.
    #[must_use]
    fn having_takes_a_number(self) -> bool {
        matches!(
            self,
            Self::Count | Self::CountDistinct | Self::Sum | Self::Avg
        )
    }

    /// The rendered expression over `column`: `count()` has no column of
    /// its own (the caller decides), every other form backticks it.
    #[must_use]
    fn render(self, column: &Ident) -> String {
        match self {
            Self::Count => format!("count(`{column}`)"),
            Self::CountDistinct => format!("count(DISTINCT `{column}`)"),
            Self::Sum => format!("sum(`{column}`)"),
            Self::Avg => format!("avg(`{column}`)"),
            Self::Min => format!("min(`{column}`)"),
            Self::Max => format!("max(`{column}`)"),
        }
    }
}

/// The right-hand side of a condition: a string literal (the default), or
/// — only where J4 demands it — a checked decimal number rendered
/// unquoted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConditionValue {
    /// A `SqlLiteral`: one string literal, self-escaping on render.
    Text(SqlLiteral),
    /// Text that passed [`is_decimal`], rendered exactly as written.
    Number(String),
}

impl std::fmt::Display for ConditionValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text(literal) => write!(f, "{literal}"),
            Self::Number(number) => f.write_str(number),
        }
    }
}

/// One checked condition of a filter or a `having` (J2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Condition {
    /// The column (a group-by column or a measure's name, for a `having`).
    pub column: Ident,
    /// One of the six operators the transform grammar allows.
    pub operator: &'static str,
    /// The right-hand side.
    pub value: ConditionValue,
}

/// One parsed output column of a join: the side, its column, and the
/// output name (the stored `name`, or the column when that was empty).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinOutputColumn {
    side: Side,
    column: Ident,
    name: Ident,
}

/// A parsed join step: everything but `left`/`right`, which the compiler
/// checks (they name an input or an earlier step, D6a).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinForm {
    join_kind: JoinKind,
    on: Vec<(Ident, Ident)>,
    columns: Vec<JoinOutputColumn>,
}

/// One parsed measure (J2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasureForm {
    function: MeasureFunction,
    column: Option<Ident>,
    name: Ident,
}

impl MeasureForm {
    /// The rendered expression (J4): `count()`, `count(`c`)`,
    /// `count(DISTINCT `c`)`, `sum(`c`)`, `avg(`c`)`, `min(`c`)`,
    /// `max(`c`)`.
    #[must_use]
    fn expression(&self) -> String {
        match self.column.as_ref() {
            Some(column) => self.function.render(column),
            // Only `count` parses without a column (MEASURE_NEEDS_COLUMN).
            None => "count()".to_owned(),
        }
    }
}

/// A parsed aggregate step (J4): everything but `reads`, which the
/// compiler checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregateForm {
    group_by: Vec<Ident>,
    measures: Vec<MeasureForm>,
    having_match: MatchMode,
    having: Vec<Condition>,
}

/// A parsed filter step (J5): everything but `reads`, which the compiler
/// checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterForm {
    match_mode: MatchMode,
    conditions: Vec<Condition>,
}

/// J2, J3: parse a join step's stored fields into a checked form.
///
/// # Errors
///
/// [`DefinitionError`] at `steps[index].joinType`, `.on[j]`, `.columns[j]`
/// or `.columns` for every rule of J2 and J3 except `left`/`right`, which
/// the compiler checks against the inputs and earlier steps (D6a).
pub fn parse_join(
    index: usize,
    join_type: &str,
    on: &[JoinPair],
    columns: &[JoinColumn],
) -> Result<JoinForm, DefinitionError> {
    let join_kind =
        JoinKind::parse(join_type).ok_or_else(|| field_err(index, "joinType", JOIN_TYPE))?;
    if on.is_empty() {
        return Err(field_err(index, "on", JOIN_NO_PAIR));
    }
    if on.len() > MAX_JOIN_PAIRS {
        return Err(field_err(index, "on", JOIN_TOO_MANY_PAIRS));
    }
    let mut pairs = Vec::with_capacity(on.len());
    for (j, pair) in on.iter().enumerate() {
        let left =
            Ident::new(pair.left.clone()).map_err(|_| row_err(index, "on", j, COLUMN_NAME))?;
        let right =
            Ident::new(pair.right.clone()).map_err(|_| row_err(index, "on", j, COLUMN_NAME))?;
        pairs.push((left, right));
    }
    if columns.is_empty() {
        return Err(field_err(index, "columns", JOIN_NO_COLUMN));
    }
    if columns.len() > MAX_JOIN_COLUMNS {
        return Err(field_err(index, "columns", JOIN_TOO_MANY_COLUMNS));
    }
    let mut outputs = Vec::with_capacity(columns.len());
    let mut seen_names: Vec<String> = Vec::with_capacity(columns.len());
    for (j, column) in columns.iter().enumerate() {
        let side =
            Side::parse(&column.side).ok_or_else(|| row_err(index, "columns", j, JOIN_SIDE))?;
        let ident = Ident::new(column.column.clone())
            .map_err(|_| row_err(index, "columns", j, COLUMN_NAME))?;
        // J3: `name` defaults to `column`; an explicit one must be as
        // plain as a column (it becomes an output column's name).
        let name = if column.name.is_empty() {
            ident.clone()
        } else {
            Ident::new(column.name.clone())
                .map_err(|_| row_err(index, "columns", j, COLUMN_NAME))?
        };
        let key = name.as_str().to_ascii_lowercase();
        if seen_names.contains(&key) {
            return Err(row_err(index, "columns", j, DUPLICATE_OUTPUT));
        }
        seen_names.push(key);
        outputs.push(JoinOutputColumn {
            side,
            column: ident,
            name,
        });
    }
    Ok(JoinForm {
        join_kind,
        on: pairs,
        columns: outputs,
    })
}

/// J2, J4: parse an aggregate step's stored fields into a checked form.
///
/// # Errors
///
/// [`DefinitionError`] at `steps[index]`, `.groupBy[j]`, `.measures[j]`,
/// `.havingMatch` or `.having[j]` for every rule of J2 and J4 except
/// `reads`, which the compiler checks against the inputs and earlier
/// steps (D6a).
pub fn parse_aggregate(
    index: usize,
    group_by: &[String],
    measures: &[Measure],
    having_match: &str,
    having: &[StepCondition],
) -> Result<AggregateForm, DefinitionError> {
    if group_by.is_empty() && measures.is_empty() {
        return Err(step_err(index, AGGREGATE_EMPTY));
    }
    if group_by.len() > MAX_GROUP_COLUMNS {
        return Err(field_err(index, "groupBy", TOO_MANY_GROUP_COLUMNS));
    }
    let mut group_columns = Vec::with_capacity(group_by.len());
    let mut output_keys: Vec<String> = Vec::with_capacity(group_by.len() + measures.len());
    for (j, name) in group_by.iter().enumerate() {
        let column =
            Ident::new(name.clone()).map_err(|_| row_err(index, "groupBy", j, COLUMN_NAME))?;
        let key = column.as_str().to_ascii_lowercase();
        if output_keys.contains(&key) {
            return Err(row_err(index, "groupBy", j, DUPLICATE_OUTPUT));
        }
        output_keys.push(key);
        group_columns.push(column);
    }
    if measures.len() > MAX_MEASURES {
        return Err(field_err(index, "measures", TOO_MANY_MEASURES));
    }
    let mut parsed_measures = Vec::with_capacity(measures.len());
    for (j, measure) in measures.iter().enumerate() {
        let function = MeasureFunction::parse(&measure.function)
            .ok_or_else(|| row_err(index, "measures", j, MEASURE_FUNCTION))?;
        // J2: `column` is optional for `count` only.
        let column = if measure.column.is_empty() {
            if function != MeasureFunction::Count {
                return Err(row_err(index, "measures", j, MEASURE_NEEDS_COLUMN));
            }
            None
        } else {
            Some(
                Ident::new(measure.column.clone())
                    .map_err(|_| row_err(index, "measures", j, COLUMN_NAME))?,
            )
        };
        // A measure's name becomes an output column's name, so it lives by
        // D2's rule (48 characters, no `__` prefix) — the same sentence,
        // reused, not a near-copy.
        if !valid_name(&measure.name) {
            return Err(row_err(
                index,
                "measures",
                j,
                crate::pipeline_compile::INVALID_NAME,
            ));
        }
        let name = Ident::new(measure.name.clone())
            .map_err(|_| row_err(index, "measures", j, crate::pipeline_compile::INVALID_NAME))?;
        let key = name.as_str().to_ascii_lowercase();
        if output_keys.contains(&key) {
            return Err(row_err(index, "measures", j, DUPLICATE_OUTPUT));
        }
        output_keys.push(key);
        parsed_measures.push(MeasureForm {
            function,
            column,
            name,
        });
    }
    // F2: a measure's name may not be any column the step names — a
    // group-by column or any measure's column — without regard to case,
    // because a `ClickHouse` alias is visible in the whole query and the
    // aggregate would read the alias back instead of the column.
    let read_keys: Vec<String> = group_columns
        .iter()
        .chain(parsed_measures.iter().filter_map(|m| m.column.as_ref()))
        .map(|column| column.as_str().to_ascii_lowercase())
        .collect();
    for (j, measure) in parsed_measures.iter().enumerate() {
        if read_keys.contains(&measure.name.as_str().to_ascii_lowercase()) {
            return Err(row_err(index, "measures", j, MEASURE_NAME_IS_COLUMN));
        }
    }
    let match_mode = MatchMode::parse(having_match)
        .ok_or_else(|| field_err(index, "havingMatch", MATCH_MODE))?;
    if having.len() > MAX_CONDITIONS {
        return Err(field_err(index, "having", TOO_MANY_CONDITIONS));
    }
    let mut parsed_having = Vec::with_capacity(having.len());
    for (j, condition) in having.iter().enumerate() {
        let mut parsed = parse_condition(index, "having", j, condition)?;
        // J4: a having names a group-by column or a measure's name, exactly
        // — and a count, distinct count, sum or average is compared with a
        // number, rendered unquoted; everything else is a string literal.
        if let Some(measure) = parsed_measures
            .iter()
            .find(|m| m.name.as_str() == parsed.column.as_str())
        {
            if measure.function.having_takes_a_number() {
                if !is_decimal(condition.value.as_str()) {
                    return Err(row_err(index, "having", j, HAVING_NOT_A_NUMBER));
                }
                parsed.value = ConditionValue::Number(condition.value.clone());
            }
        } else if !group_columns
            .iter()
            .any(|column| column.as_str() == parsed.column.as_str())
        {
            return Err(row_err(index, "having", j, HAVING_UNKNOWN));
        }
        parsed_having.push(parsed);
    }
    Ok(AggregateForm {
        group_by: group_columns,
        measures: parsed_measures,
        having_match: match_mode,
        having: parsed_having,
    })
}

/// J2, J5: parse a filter step's stored fields into a checked form.
///
/// # Errors
///
/// [`DefinitionError`] at `steps[index].match`, `.conditions` or
/// `.conditions[j]` for every rule of J2 and J5 except `reads`, which the
/// compiler checks against the inputs and earlier steps (D6a).
pub fn parse_filter(
    index: usize,
    match_mode: &str,
    conditions: &[StepCondition],
) -> Result<FilterForm, DefinitionError> {
    let mode = MatchMode::parse(match_mode).ok_or_else(|| field_err(index, "match", MATCH_MODE))?;
    if conditions.is_empty() {
        return Err(field_err(index, "conditions", NO_CONDITION));
    }
    if conditions.len() > MAX_CONDITIONS {
        return Err(field_err(index, "conditions", TOO_MANY_CONDITIONS));
    }
    let parsed = conditions
        .iter()
        .enumerate()
        .map(|(j, condition)| parse_condition(index, "conditions", j, condition))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(FilterForm {
        match_mode: mode,
        conditions: parsed,
    })
}

/// J2: parse one condition — a checked column, one of the six operators
/// the transform grammar allows (through [`filter_operator`], the one
/// list), and a value of at most [`MAX_VALUE_CHARS`] characters carried as
/// a `SqlLiteral`.
///
/// # Errors
///
/// [`DefinitionError`] at `steps[index].<field>[j]` when the column is
/// not a plain name, the operator is not one of the six, or the value is
/// too long.
fn parse_condition(
    index: usize,
    field: &str,
    j: usize,
    condition: &StepCondition,
) -> Result<Condition, DefinitionError> {
    let column =
        Ident::new(condition.column.clone()).map_err(|_| row_err(index, field, j, COLUMN_NAME))?;
    let operator =
        filter_operator(&condition.operator).ok_or_else(|| row_err(index, field, j, OPERATOR))?;
    if condition.value.chars().count() > MAX_VALUE_CHARS {
        return Err(row_err(index, field, j, VALUE_TOO_LONG));
    }
    Ok(Condition {
        column,
        operator,
        value: ConditionValue::Text(SqlLiteral::from(condition.value.clone())),
    })
}

/// J4: whether `text` is a decimal number — an optional `-`, 1 to 30
/// digits, optionally a `.` and 1 to 30 more digits — checked character by
/// character, so no `regex` dependency is added for one shape.
///
/// This is the only gate a value passes to be rendered unquoted; every
/// other value is a `SqlLiteral`.
#[must_use]
pub fn is_decimal(text: &str) -> bool {
    let text = text.strip_prefix('-').unwrap_or(text);
    match text.split_once('.') {
        None => decimal_digits(text),
        Some((int_part, rest)) => {
            decimal_digits(int_part) && !rest.contains('.') && decimal_digits(rest)
        }
    }
}

/// [`is_decimal`]'s one part: 1 to [`MAX_NUMBER_DIGITS`] digits and nothing
/// else.
fn decimal_digits(part: &str) -> bool {
    let count = part.chars().count();
    (1..=MAX_NUMBER_DIGITS).contains(&count) && part.chars().all(|c| c.is_ascii_digit())
}

/// J3: render a parsed join as its CTE body — the listed output columns,
/// the two sides under the fixed `__left`/`__right` aliases, and the
/// equality pairs joined by ` AND `.
///
/// `left_name` and `right_name` are the step's stored `left`/`right`,
/// already checked by the compiler (they equal an input's or an earlier
/// step's name, which passed D2's rule), so this function formats only
/// checked values.
#[must_use]
pub fn render_join(left_name: &str, right_name: &str, form: &JoinForm) -> String {
    let columns = form
        .columns
        .iter()
        .map(|column| {
            format!(
                "`{}`.`{}` AS `{}`",
                column.side.alias(),
                column.column,
                column.name
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let pairs = form
        .on
        .iter()
        .map(|(left, right)| format!("`__left`.`{left}` = `__right`.`{right}`"))
        .collect::<Vec<_>>()
        .join(" AND ");
    format!(
        "SELECT {columns} FROM `{left_name}` AS `__left` {} `{right_name}` AS `__right` ON {pairs}",
        form.join_kind.keyword()
    )
}

/// J4: render a parsed aggregate as its CTE body — the group-by columns
/// as themselves, each measure as its expression under its name, and the
/// `having` conditions with the measure's EXPRESSION (never its alias, so
/// F2's alias question cannot arise).
///
/// `reads` is the step's stored read, already checked by the compiler.
#[must_use]
pub fn render_aggregate(reads: &str, form: &AggregateForm) -> String {
    let mut select: Vec<String> = form
        .group_by
        .iter()
        .map(|column| format!("`{column}`"))
        .collect();
    select.extend(
        form.measures
            .iter()
            .map(|measure| format!("{} AS `{}`", measure.expression(), measure.name)),
    );
    let mut sql = format!("SELECT {} FROM `{reads}`", select.join(", "));
    if !form.group_by.is_empty() {
        let group = form
            .group_by
            .iter()
            .map(|column| format!("`{column}`"))
            .collect::<Vec<_>>()
            .join(", ");
        sql.push_str(" GROUP BY ");
        sql.push_str(&group);
    }
    if !form.having.is_empty() {
        let conditions = form
            .having
            .iter()
            .map(|condition| {
                // A having names a group-by column or a measure's name,
                // exactly (parse proved it); the left-hand side is that
                // column, or the measure's expression, not the alias.
                let side = form
                    .group_by
                    .iter()
                    .find(|column| column.as_str() == condition.column.as_str())
                    .map_or_else(
                        || {
                            form.measures
                                .iter()
                                .find(|measure| measure.name.as_str() == condition.column.as_str())
                                .map_or_else(
                                    || format!("`{}`", condition.column),
                                    MeasureForm::expression,
                                )
                        },
                        |column| format!("`{column}`"),
                    );
                format!("{side} {} {}", condition.operator, condition.value)
            })
            .collect::<Vec<_>>()
            .join(form.having_match.joiner());
        sql.push_str(" HAVING ");
        sql.push_str(&conditions);
    }
    sql
}

/// J5: render a parsed filter as its CTE body — `SELECT *` over what it
/// reads, keeping the rows that meet all, or any, of its conditions.
///
/// `reads` is the step's stored read, already checked by the compiler.
#[must_use]
pub fn render_filter(reads: &str, form: &FilterForm) -> String {
    let conditions = form
        .conditions
        .iter()
        .map(|condition| {
            format!(
                "`{}` {} {}",
                condition.column, condition.operator, condition.value
            )
        })
        .collect::<Vec<_>>()
        .join(form.match_mode.joiner());
    format!("SELECT * FROM `{reads}` WHERE {conditions}")
}

#[cfg(test)]
mod tests {
    //! The plan's S1-T2 check: the rendered text pinned byte for byte, every
    //! refusal asserting its exact path and constant, and security rule 5
    //! (a hostile value stays one string literal to `sqlparser`, what the
    //! gate and the rewrite see).

    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use lakehouse_store::pipelines::{JoinColumn, JoinPair, Measure, StepCondition};

    use super::{
        ConditionValue, FilterForm, JoinForm, MeasureFunction, parse_aggregate, parse_filter,
        parse_join, render_aggregate, render_filter, render_join,
    };
    use crate::pipeline_compile::DefinitionError;

    /// The fixed sentences, by hand in one place so a changed constant fails
    /// the test that names it, not every assertion in this module.
    const JOIN_TYPE: &str = "a join is inner, left, right or full";
    const JOIN_NO_PAIR: &str = "a join needs at least one pair of columns to match on";
    const JOIN_TOO_MANY_PAIRS: &str = "a join matches on at most 10 pairs of columns";
    const JOIN_NO_COLUMN: &str = "a join needs at least one output column";
    const JOIN_TOO_MANY_COLUMNS: &str = "a join has at most 300 output columns";
    const JOIN_SIDE: &str = "an output column comes from the left or the right side";
    const COLUMN_NAME: &str = "a column must be a plain name (letters, digits and underscores, not starting with a digit)";
    const DUPLICATE_OUTPUT: &str =
        "another output column of this step has this name; give one of them another name";
    const MATCH_MODE: &str = "match is all or any";
    const NO_CONDITION: &str = "a filter needs at least one condition";
    const TOO_MANY_CONDITIONS: &str = "a step has at most 20 conditions";
    const OPERATOR: &str = "a condition's operator is one of = != < > <= >=";
    const VALUE_TOO_LONG: &str = "a condition's value is at most 1,000 characters";
    const AGGREGATE_EMPTY: &str = "an aggregation needs a column to group by or a measure";
    const TOO_MANY_GROUP_COLUMNS: &str = "an aggregation groups by at most 20 columns";
    const TOO_MANY_MEASURES: &str = "an aggregation has at most 50 measures";
    const MEASURE_FUNCTION: &str = "a measure is count, countDistinct, sum, avg, min or max";
    const MEASURE_NEEDS_COLUMN: &str = "this measure needs a column";
    const MEASURE_NAME_IS_COLUMN: &str =
        "a measure's name cannot be a column this step reads; choose another name";
    const HAVING_UNKNOWN: &str =
        "a condition here names a group-by column or a measure of this step";
    const HAVING_NOT_A_NUMBER: &str = "a count, sum or average is compared with a number";
    const INVALID_NAME: &str = "a name must be letters, digits and underscores (not starting with a digit or __), \
         at most 48 characters";

    fn pair(left: &str, right: &str) -> JoinPair {
        JoinPair {
            left: left.to_owned(),
            right: right.to_owned(),
        }
    }

    fn output_column(side: &str, column: &str, name: &str) -> JoinColumn {
        JoinColumn {
            side: side.to_owned(),
            column: column.to_owned(),
            name: name.to_owned(),
        }
    }

    fn condition(column: &str, operator: &str, value: &str) -> StepCondition {
        StepCondition {
            column: column.to_owned(),
            operator: operator.to_owned(),
            value: value.to_owned(),
        }
    }

    fn measure(function: &str, column: &str, name: &str) -> Measure {
        Measure {
            function: function.to_owned(),
            column: column.to_owned(),
            name: name.to_owned(),
        }
    }

    /// One refusal, asserted by path AND constant (D7: the console selects
    /// the step and row from the path; the sentence is what the person
    /// reads).
    fn refused(err: &DefinitionError, path: &str, message: &str) {
        assert_eq!(err.path(), path, "{message}");
        assert_eq!(err.message(), message, "{path}");
    }

    /// J2's join, parsed once for the tests that render it.
    fn j2_join() -> JoinForm {
        parse_join(
            0,
            "left",
            &[pair("customer_id", "id")],
            &[
                output_column("left", "order_id", ""),
                output_column("left", "amount", ""),
                output_column("right", "country", ""),
                output_column("right", "id", "customer_ref"),
            ],
        )
        .unwrap()
    }

    // ── the rendered text, byte for byte ────────────────────────────────

    #[test]
    fn a_left_join_renders_the_listed_columns_and_the_pairs() {
        assert_eq!(
            render_join("orders", "customers", &j2_join()),
            "SELECT `__left`.`order_id` AS `order_id`, `__left`.`amount` AS `amount`, \
             `__right`.`country` AS `country`, `__right`.`id` AS `customer_ref` \
             FROM `orders` AS `__left` LEFT JOIN `customers` AS `__right` \
             ON `__left`.`customer_id` = `__right`.`id`"
        );
    }

    #[test]
    fn each_join_type_renders_its_keyword() {
        for (stored, keyword) in [
            ("inner", "INNER JOIN"),
            ("left", "LEFT JOIN"),
            ("right", "RIGHT JOIN"),
            ("full", "FULL OUTER JOIN"),
        ] {
            let form = parse_join(
                0,
                stored,
                &[pair("a", "b")],
                &[output_column("left", "c", "")],
            )
            .unwrap();
            let rendered = render_join("l", "r", &form);
            assert!(
                rendered.contains(&format!("{keyword} `r` AS `__right`")),
                "{stored}: {rendered}"
            );
        }
    }

    #[test]
    fn two_pairs_are_joined_by_and() {
        let form = parse_join(
            0,
            "inner",
            &[pair("a", "b"), pair("c", "d")],
            &[output_column("left", "e", "")],
        )
        .unwrap();
        assert!(
            render_join("l", "r", &form)
                .ends_with("ON `__left`.`a` = `__right`.`b` AND `__left`.`c` = `__right`.`d`"),
            "pairs are joined by AND"
        );
    }

    #[test]
    fn a_self_join_uses_two_aliases() {
        let form = parse_join(
            0,
            "inner",
            &[pair("id", "parent_id")],
            &[
                output_column("left", "id", ""),
                output_column("right", "id", "parent"),
            ],
        )
        .unwrap();
        assert_eq!(
            render_join("tree", "tree", &form),
            "SELECT `__left`.`id` AS `id`, `__right`.`id` AS `parent` \
             FROM `tree` AS `__left` INNER JOIN `tree` AS `__right` \
             ON `__left`.`id` = `__right`.`parent_id`"
        );
    }

    /// J2's aggregate: group-by, a sum and a `count()`, and a `having` that
    /// repeats the measure's EXPRESSION rather than its alias (F2 cannot
    /// arise in a `HAVING` this module writes).
    #[test]
    fn an_aggregate_renders_group_by_measures_and_having_with_the_expression() {
        let form = parse_aggregate(
            0,
            &["country".to_owned()],
            &[
                measure("sum", "amount", "revenue"),
                measure("count", "", "order_count"),
            ],
            "all",
            &[condition("revenue", ">", "1000")],
        )
        .unwrap();
        assert_eq!(
            render_aggregate("joined", &form),
            "SELECT `country`, sum(`amount`) AS `revenue`, count() AS `order_count` \
             FROM `joined` GROUP BY `country` HAVING sum(`amount`) > 1000"
        );
    }

    #[test]
    fn each_measure_function_renders_its_expression() {
        for (function, column, expression) in [
            ("count", "", "count()"),
            ("count", "c", "count(`c`)"),
            ("countDistinct", "c", "count(DISTINCT `c`)"),
            ("sum", "c", "sum(`c`)"),
            ("avg", "c", "avg(`c`)"),
            ("min", "c", "min(`c`)"),
            ("max", "c", "max(`c`)"),
        ] {
            let form = parse_aggregate(0, &[], &[measure(function, column, "m")], "", &[]).unwrap();
            assert_eq!(
                render_aggregate("t", &form),
                format!("SELECT {expression} AS `m` FROM `t`"),
                "{function}"
            );
        }
    }

    #[test]
    fn an_aggregate_with_no_group_by_has_no_group_by_clause() {
        let form = parse_aggregate(0, &[], &[measure("count", "", "n")], "", &[]).unwrap();
        assert_eq!(
            render_aggregate("t", &form),
            "SELECT count() AS `n` FROM `t`"
        );
    }

    #[test]
    fn a_filter_with_any_joins_by_or() {
        let form = parse_filter(
            0,
            "any",
            &[
                condition("revenue", ">=", "5000"),
                condition("country", "=", "ID"),
            ],
        )
        .unwrap();
        assert_eq!(
            render_filter("by_country", &form),
            "SELECT * FROM `by_country` WHERE `revenue` >= '5000' OR `country` = 'ID'"
        );
    }

    #[test]
    fn a_filter_with_all_joins_by_and() {
        let form = parse_filter(
            0,
            "all",
            &[
                condition("revenue", ">=", "5000"),
                condition("country", "=", "ID"),
            ],
        )
        .unwrap();
        assert_eq!(
            render_filter("by_country", &form),
            "SELECT * FROM `by_country` WHERE `revenue` >= '5000' AND `country` = 'ID'"
        );
    }

    #[test]
    fn a_missing_match_means_all() {
        for stored in ["", "all"] {
            let form = parse_filter(
                0,
                stored,
                &[condition("a", "=", "1"), condition("b", "=", "2")],
            )
            .unwrap();
            assert!(
                render_filter("t", &form).contains("'1' AND `b`"),
                "empty match is accepted and means all: {stored}"
            );
        }
    }

    // ── refusals: the exact path and constant ───────────────────────────

    #[test]
    fn a_join_type_outside_the_four_is_refused_at_join_type() {
        let err = parse_join(
            0,
            "cross",
            &[pair("a", "b")],
            &[output_column("left", "c", "")],
        )
        .unwrap_err();
        refused(&err, "steps[0].joinType", JOIN_TYPE);
    }

    #[test]
    fn a_join_with_no_pair_is_refused_at_on() {
        let err = parse_join(0, "inner", &[], &[output_column("left", "c", "")]).unwrap_err();
        refused(&err, "steps[0].on", JOIN_NO_PAIR);
    }

    #[test]
    fn a_join_with_eleven_pairs_is_refused_at_on() {
        let pairs: Vec<JoinPair> = (0..11).map(|i| pair(&format!("a{i}"), "b")).collect();
        let err = parse_join(0, "inner", &pairs, &[output_column("left", "c", "")]).unwrap_err();
        refused(&err, "steps[0].on", JOIN_TOO_MANY_PAIRS);
    }

    #[test]
    fn a_join_with_no_output_column_is_refused_at_columns() {
        let err = parse_join(0, "inner", &[pair("a", "b")], &[]).unwrap_err();
        refused(&err, "steps[0].columns", JOIN_NO_COLUMN);
    }

    #[test]
    fn a_join_with_301_output_columns_is_refused_at_columns() {
        let columns: Vec<JoinColumn> = (0..301)
            .map(|i| output_column("left", &format!("c{i}"), ""))
            .collect();
        let err = parse_join(0, "inner", &[pair("a", "b")], &columns).unwrap_err();
        refused(&err, "steps[0].columns", JOIN_TOO_MANY_COLUMNS);
    }

    #[test]
    fn a_column_from_the_middle_is_refused_at_its_row() {
        let err = parse_join(
            0,
            "inner",
            &[pair("a", "b")],
            &[output_column("middle", "c", "")],
        )
        .unwrap_err();
        refused(&err, "steps[0].columns[0]", JOIN_SIDE);
    }

    /// A column that is not a plain name is refused with `COLUMN_NAME` in
    /// every field that holds one — the plan's S1-T2 matrix: `on`,
    /// `columns` (column and output name), `groupBy`, `measures`,
    /// `conditions`, `having`.
    #[test]
    fn a_column_that_is_not_a_plain_name_is_refused_in_every_field() {
        for bad in ["amount; DROP TABLE x", "a`b", "2col", ""] {
            // on[j]
            refused(
                &parse_join(
                    0,
                    "inner",
                    &[pair(bad, "b")],
                    &[output_column("left", "c", "")],
                )
                .unwrap_err(),
                "steps[0].on[0]",
                COLUMN_NAME,
            );
            // columns[j].column
            refused(
                &parse_join(
                    0,
                    "inner",
                    &[pair("a", "b")],
                    &[output_column("left", bad, "")],
                )
                .unwrap_err(),
                "steps[0].columns[0]",
                COLUMN_NAME,
            );
            // columns[j].name (explicit, not the default; an EMPTY name
            // means "use the column", J3, so this row runs on the
            // non-empty bad names only)
            if !bad.is_empty() {
                refused(
                    &parse_join(
                        0,
                        "inner",
                        &[pair("a", "b")],
                        &[output_column("left", "c", bad)],
                    )
                    .unwrap_err(),
                    "steps[0].columns[0]",
                    COLUMN_NAME,
                );
            }
            // groupBy[j]
            refused(
                &parse_aggregate(0, &[bad.to_owned()], &[measure("count", "", "n")], "", &[])
                    .unwrap_err(),
                "steps[0].groupBy[0]",
                COLUMN_NAME,
            );
            // measures[j].column — an EMPTY column of a `sum` is refused as
            // MEASURE_NEEDS_COLUMN first (J2), so this row runs on the
            // non-empty bad names only.
            if !bad.is_empty() {
                refused(
                    &parse_aggregate(0, &[], &[measure("sum", bad, "m")], "", &[]).unwrap_err(),
                    "steps[0].measures[0]",
                    COLUMN_NAME,
                );
            }
            // conditions[j].column
            refused(
                &parse_filter(0, "all", &[condition(bad, "=", "1")]).unwrap_err(),
                "steps[0].conditions[0]",
                COLUMN_NAME,
            );
            // having[j].column
            refused(
                &parse_aggregate(
                    0,
                    &["country".to_owned()],
                    &[measure("count", "", "n")],
                    "",
                    &[condition(bad, "=", "1")],
                )
                .unwrap_err(),
                "steps[0].having[0]",
                COLUMN_NAME,
            );
        }
    }

    #[test]
    fn two_output_columns_differing_only_in_case_are_refused() {
        let err = parse_join(
            0,
            "inner",
            &[pair("a", "b")],
            &[
                output_column("left", "id", ""),
                output_column("right", "ID", ""),
            ],
        )
        .unwrap_err();
        refused(&err, "steps[0].columns[1]", DUPLICATE_OUTPUT);
    }

    #[test]
    fn an_unknown_measure_function_is_refused() {
        let err =
            parse_aggregate(0, &[], &[measure("median", "amount", "m")], "", &[]).unwrap_err();
        refused(&err, "steps[0].measures[0]", MEASURE_FUNCTION);
    }

    #[test]
    fn a_sum_without_a_column_is_refused() {
        let err = parse_aggregate(0, &[], &[measure("sum", "", "m")], "", &[]).unwrap_err();
        refused(&err, "steps[0].measures[0]", MEASURE_NEEDS_COLUMN);
    }

    /// F2: a measure named like the column it sums — `ClickHouse` would
    /// read the alias back and make the query `sum(sum(amount))`.
    #[test]
    fn a_measure_named_like_its_column_is_refused() {
        let err =
            parse_aggregate(0, &[], &[measure("sum", "amount", "amount")], "", &[]).unwrap_err();
        refused(&err, "steps[0].measures[0]", MEASURE_NAME_IS_COLUMN);
    }

    /// J4's own pin: a measure named like a group-by column is a duplicate
    /// OUTPUT first (`country` is already an output column), so the
    /// duplicate sentence is the one a person sees.
    #[test]
    fn a_measure_named_like_a_group_by_column_is_a_duplicate_output() {
        let err = parse_aggregate(
            0,
            &["country".to_owned()],
            &[measure("count", "", "country")],
            "",
            &[],
        )
        .unwrap_err();
        refused(&err, "steps[0].measures[0]", DUPLICATE_OUTPUT);
    }

    #[test]
    fn a_having_that_names_no_output_of_the_step_is_refused() {
        let err = parse_aggregate(
            0,
            &["country".to_owned()],
            &[measure("sum", "amount", "revenue")],
            "",
            &[condition("nope", "=", "1")],
        )
        .unwrap_err();
        refused(&err, "steps[0].having[0]", HAVING_UNKNOWN);
    }

    /// J4: a `sum`'s having value is a number, so an injection-shaped text
    /// is refused as not a number — never rendered, never escaped into one.
    #[test]
    fn a_having_on_a_sum_with_a_text_value_is_refused_as_not_a_number() {
        let err = parse_aggregate(
            0,
            &[],
            &[measure("sum", "amount", "revenue")],
            "",
            &[condition("revenue", ">", "1000 OR 1=1")],
        )
        .unwrap_err();
        refused(&err, "steps[0].having[0]", HAVING_NOT_A_NUMBER);
    }

    #[test]
    fn an_operator_outside_the_six_is_refused() {
        let err = parse_filter(0, "all", &[condition("c", "LIKE", "x")]).unwrap_err();
        refused(&err, "steps[0].conditions[0]", OPERATOR);
    }

    #[test]
    fn a_value_of_1001_characters_is_refused_and_1000_is_not() {
        let long = "x".repeat(1_001);
        refused(
            &parse_filter(0, "all", &[condition("c", "=", &long)]).unwrap_err(),
            "steps[0].conditions[0]",
            VALUE_TOO_LONG,
        );
        let exact = "x".repeat(1_000);
        assert!(parse_filter(0, "all", &[condition("c", "=", &exact)]).is_ok());
    }

    #[test]
    fn twenty_one_conditions_are_refused() {
        let conditions: Vec<StepCondition> = (0..21)
            .map(|i| condition(&format!("c{i}"), "=", "1"))
            .collect();
        refused(
            &parse_filter(0, "all", &conditions).unwrap_err(),
            "steps[0].conditions",
            TOO_MANY_CONDITIONS,
        );
        let twenty: Vec<StepCondition> = (0..20)
            .map(|i| condition(&format!("c{i}"), "=", "1"))
            .collect();
        assert!(parse_filter(0, "all", &twenty).is_ok());
    }

    #[test]
    fn an_aggregate_with_neither_group_by_nor_measures_is_refused() {
        refused(
            &parse_aggregate(0, &[], &[], "", &[]).unwrap_err(),
            "steps[0]",
            AGGREGATE_EMPTY,
        );
    }

    #[test]
    fn twenty_one_group_by_columns_are_refused() {
        let columns: Vec<String> = (0..21).map(|i| format!("c{i}")).collect();
        refused(
            &parse_aggregate(0, &columns, &[measure("count", "", "n")], "", &[]).unwrap_err(),
            "steps[0].groupBy",
            TOO_MANY_GROUP_COLUMNS,
        );
    }

    #[test]
    fn fifty_one_measures_are_refused() {
        let measures: Vec<Measure> = (0..51)
            .map(|i| measure("count", "", &format!("n{i}")))
            .collect();
        refused(
            &parse_aggregate(0, &[], &measures, "", &[]).unwrap_err(),
            "steps[0].measures",
            TOO_MANY_MEASURES,
        );
    }

    #[test]
    fn a_match_outside_all_or_any_is_refused() {
        refused(
            &parse_filter(0, "some", &[condition("c", "=", "1")]).unwrap_err(),
            "steps[0].match",
            MATCH_MODE,
        );
        refused(
            &parse_aggregate(
                0,
                &["c".to_owned()],
                &[],
                "some",
                &[condition("c", "=", "1")],
            )
            .unwrap_err(),
            "steps[0].havingMatch",
            MATCH_MODE,
        );
    }

    #[test]
    fn a_filter_with_no_condition_is_refused() {
        refused(
            &parse_filter(0, "all", &[]).unwrap_err(),
            "steps[0].conditions",
            NO_CONDITION,
        );
    }

    /// A measure's name lives by D2's rule (it becomes an output column
    /// the next step reads by), so it reuses D2's own sentence.
    #[test]
    fn a_measure_name_that_breaks_d2s_rule_is_refused() {
        refused(
            &parse_aggregate(0, &[], &[measure("count", "", "__reserved")], "", &[]).unwrap_err(),
            "steps[0].measures[0]",
            INVALID_NAME,
        );
        let long = "n".repeat(49);
        refused(
            &parse_aggregate(0, &[], &[measure("count", "", &long)], "", &[]).unwrap_err(),
            "steps[0].measures[0]",
            INVALID_NAME,
        );
    }

    /// Twenty-one `having` conditions ride the same cap as a filter's.
    #[test]
    fn twenty_one_having_conditions_are_refused() {
        let having: Vec<StepCondition> = (0..21)
            .map(|i| condition("country", "=", &format!("{i}")))
            .collect();
        refused(
            &parse_aggregate(0, &["country".to_owned()], &[], "", &having).unwrap_err(),
            "steps[0].having",
            TOO_MANY_CONDITIONS,
        );
    }

    // ── security rule 5: a hostile value is one string literal ──────────

    /// For each hostile value: the rendered filter parses as ONE query
    /// whose `WHERE` is one `BinaryOp` with a `Value::SingleQuotedString`
    /// right side equal to the original value — one string literal to
    /// `sqlparser` (what the gate and the rewrite see) and in the text.
    #[test]
    fn a_hostile_value_is_one_string_literal() {
        use sqlparser::ast::{Expr, SetExpr, Statement, Value};
        use sqlparser::dialect::ClickHouseDialect;
        use sqlparser::parser::Parser;

        for hostile in [
            "x' OR '1'='1",
            "a\\",
            "a\\'b",
            "-- c",
            "/* c",
            ";DROP TABLE t",
            "line\nbreak",
        ] {
            let form: FilterForm = parse_filter(0, "all", &[condition("c", "=", hostile)]).unwrap();
            let rendered = render_filter("t", &form);
            let statements =
                Parser::parse_sql(&ClickHouseDialect {}, &rendered).expect("it parses");
            let [Statement::Query(query)] = statements.as_slice() else {
                panic!("one statement for {hostile:?}: {rendered}");
            };
            let SetExpr::Select(select) = query.body.as_ref() else {
                panic!("one select for {hostile:?}");
            };
            let Some(Expr::BinaryOp { right, .. }) = &select.selection else {
                panic!("one comparison for {hostile:?}: {rendered}");
            };
            let Expr::Value(literal) = right.as_ref() else {
                panic!("a quoted literal for {hostile:?}: {rendered}");
            };
            let Value::SingleQuotedString(text) = &literal.value else {
                panic!("a quoted literal for {hostile:?}: {rendered}");
            };
            assert_eq!(text, hostile, "the literal survives round trip: {rendered}");
        }
    }

    // ── is_decimal (J4) ────────────────────────────────────────────────

    #[test]
    fn is_decimal_accepts_plain_numbers() {
        assert!(super::is_decimal("1000"));
        assert!(super::is_decimal("-2.5"));
        assert!(super::is_decimal("0"));
        assert!(super::is_decimal("-0.000000000000000000000000000001"));
    }

    #[test]
    fn is_decimal_refuses_everything_else() {
        for not_a_number in [
            "1e3", "1.", ".5", "1 OR 1", "", "1.2.3", "-", "1-2", "1 000", "0x10",
        ] {
            assert!(
                !super::is_decimal(not_a_number),
                "{not_a_number:?} is not a decimal"
            );
        }
        let thirty_one = "1".repeat(31);
        assert!(!super::is_decimal(&thirty_one));
        let thirty = "1".repeat(30);
        assert!(super::is_decimal(&thirty));
    }

    /// A having on a `count`, `countDistinct`, `sum` or `avg` carries its
    /// value as a checked number; on a `min`, a `max` or a group-by column
    /// it stays a string literal (J4).
    #[test]
    fn a_having_value_is_a_number_only_where_j4_says_so() {
        for (function, number) in [
            ("count", true),
            ("countDistinct", true),
            ("sum", true),
            ("avg", true),
            ("min", false),
            ("max", false),
        ] {
            let form = parse_aggregate(
                0,
                &[],
                &[measure(function, "c", "m")],
                "",
                &[condition("m", ">", "1000")],
            )
            .unwrap();
            let is_number = matches!(form.having[0].value, ConditionValue::Number(_));
            assert_eq!(is_number, number, "{function}");
        }
        let grouped = parse_aggregate(
            0,
            &["country".to_owned()],
            &[measure("count", "", "n")],
            "",
            &[condition("country", "=", "ID")],
        )
        .unwrap();
        assert!(matches!(grouped.having[0].value, ConditionValue::Text(_)));
    }

    /// F2's guard rail, from the other side: a measure's name that is NOT a
    /// column the step reads is accepted, and so is an aggregate whose
    /// measure reads the column another measure is named after — only the
    /// NAME is refused, not the coincidence.
    #[test]
    fn a_measure_name_that_is_no_column_is_accepted() {
        use lakehouse_core::ident::Ident;
        let form = parse_aggregate(
            0,
            &["country".to_owned()],
            &[
                measure("sum", "amount", "revenue"),
                measure("count", "", "orders_n"),
            ],
            "",
            &[condition("revenue", ">=", "10")],
        )
        .unwrap();
        assert_eq!(form.measures[0].name, Ident::new("revenue").unwrap());
        assert!(matches!(
            MeasureFunction::parse("countDistinct"),
            Some(MeasureFunction::CountDistinct)
        ));
        assert!(matches!(form.having[0].value, ConditionValue::Number(_)));
    }
}
