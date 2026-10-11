//! The checker and the SQL generator, in one pass.
//!
//! [`compile`] walks the syntax tree once and, for every node, decides its
//! type ([`FType`]), its level ([`Level`]: a constant, a value per row, or an
//! aggregate over rows) and its SQL. Checking and generating in one pass is
//! what keeps "the formula is valid" and "this is the statement" from ever
//! disagreeing.
//!
//! # Levels
//!
//! A value per row (`[revenue] - [cost]`) can be a dimension, or be
//! aggregated by a chart. An aggregate (`Sum([revenue]) / CountDistinct([id])`)
//! is a measure on its own. Mixing a bare row-level column with an aggregate
//! (`Sum([a]) / [b]`) has no meaning in a grouped query and is refused at the
//! column, with its position. An aggregate inside an aggregate is refused too.
//!
//! # What is written into the SQL, and from what
//!
//! See the module doc of [`super`]. In this file: function names come from
//! the `match` arms below, never from the formula's text; a column name is
//! written only after it is found among the source's own columns and passes
//! [`Ident::new`]; text goes through [`SqlLiteral`]; numbers go through
//! [`print_number`]; units (`'month'`) are looked up in a closed list and the
//! list's own word is written, not the person's. `/` and `Mod` give an empty
//! value for a zero divisor instead of an engine error or an infinity.

use std::collections::{BTreeMap, BTreeSet};

use lakehouse_core::ident::{Ident, SqlLiteral};
use serde::Serialize;

use super::catalog::{self, FnInfo};
use super::parse::{BinOp, Kind, Node, parse};
use super::{FormulaError, clip};
use crate::builder::RelationColumns;
use crate::filters::ColumnKind;
use crate::grain::{Grain, TimeContext};

/// How many fields one formula may reach through other fields.
pub const MAX_FIELD_CHAIN: usize = 8;
/// Most SQL bytes that field references may expand into, in total.
const MAX_EXPANDED_SQL: usize = 400_000;
/// Units a date function accepts.
const UNITS: [&str; 7] = ["minute", "hour", "day", "week", "month", "quarter", "year"];

/// The type of a value in a formula.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FType {
    /// Any number.
    Number,
    /// Text.
    Text,
    /// A calendar date.
    Date,
    /// A date with a time.
    DateTime,
    /// True or false.
    Boolean,
}

impl FType {
    /// The wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Number => "number",
            Self::Text => "text",
            Self::Date => "date",
            Self::DateTime => "datetime",
            Self::Boolean => "boolean",
        }
    }

    /// Parse a stored type name.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        [
            Self::Number,
            Self::Text,
            Self::Date,
            Self::DateTime,
            Self::Boolean,
        ]
        .into_iter()
        .find(|t| t.as_str() == raw)
    }

    /// The column kind a chart treats a field of this type as (a boolean is
    /// compared as text, like a boolean column).
    #[must_use]
    pub fn column_kind(self) -> ColumnKind {
        match self {
            Self::Number => ColumnKind::Number,
            Self::Date => ColumnKind::Date,
            Self::DateTime => ColumnKind::DateTime,
            Self::Text | Self::Boolean => ColumnKind::Text,
        }
    }

    fn of(kind: ColumnKind) -> Self {
        match kind {
            ColumnKind::Number => Self::Number,
            ColumnKind::Text => Self::Text,
            ColumnKind::Date => Self::Date,
            ColumnKind::DateTime => Self::DateTime,
        }
    }

    fn word(self) -> &'static str {
        match self {
            Self::Number => "a number",
            Self::Text => "text",
            Self::Date => "a date",
            Self::DateTime => "a timestamp",
            Self::Boolean => "true or false",
        }
    }

    fn is_temporal(self) -> bool {
        matches!(self, Self::Date | Self::DateTime)
    }
}

/// Whether a value is fixed, one per row, or one per group of rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// A literal or `Today()`: fits either side.
    Constant,
    /// One value per row.
    Row,
    /// One value per group of rows.
    Aggregate,
    /// A value computed over the whole grouped result of a chart (a running
    /// total, a rank, the previous period): `BI-8` part 2.
    Table,
}

impl Level {
    /// The wire name: a constant is stored as a row-level formula.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Aggregate => "aggregate",
            Self::Table => "table",
            Self::Constant | Self::Row => "row",
        }
    }

    /// Parse a stored level name.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "row" => Some(Self::Row),
            "aggregate" => Some(Self::Aggregate),
            "table" => Some(Self::Table),
            _ => None,
        }
    }
}

/// What a formula may name: the source's columns, its other fields, and the
/// report time settings.
pub struct Scope<'a> {
    columns: &'a RelationColumns,
    fields: BTreeMap<String, String>,
    time: &'a TimeContext,
    chart: Option<ChartCtx>,
}

/// What a formula needs to know about the chart it is built into, for the
/// functions that depend on it (table calculations, period comparisons and
/// `Fixed`, `BI-8` part 2). Without one (saving a field, a test) those
/// functions are checked for shape only.
#[derive(Debug, Clone)]
pub struct ChartCtx {
    /// The chart's dimension column (or field), if it has one.
    pub dimension: Option<String>,
    /// The chart's breakdown column, which a table calculation partitions by.
    pub breakdown: Option<String>,
    /// The expression the dimension is ordered by (`d`, or `d % 7` for a
    /// Sunday-first day of week).
    pub order_key: String,
    /// The chart's grain, if it has one.
    pub grain: Option<Grain>,
}

impl<'a> Scope<'a> {
    /// A scope over `columns` with no fields yet.
    #[must_use]
    pub fn new(columns: &'a RelationColumns, time: &'a TimeContext) -> Self {
        Self {
            columns,
            fields: BTreeMap::new(),
            time,
            chart: None,
        }
    }

    /// Build the formula into this chart.
    #[must_use]
    pub fn with_chart(mut self, chart: ChartCtx) -> Self {
        self.chart = Some(chart);
        self
    }

    /// Make the field `name` (its formula text) nameable.
    #[must_use]
    pub fn with_field(mut self, name: &str, formula: &str) -> Self {
        self.fields.insert(name.to_owned(), formula.to_owned());
        self
    }

    /// Whether `name` is a column of the source.
    #[must_use]
    pub fn has_column(&self, name: &str) -> bool {
        self.columns.contains_key(name)
    }
}

/// A formula that checks out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compiled {
    /// The `ClickHouse` expression.
    pub sql: String,
    /// Its type.
    pub ty: FType,
    /// Its level (a constant reports as `Row`).
    pub level: Level,
    /// The source's columns the expression reads, sorted.
    pub reads: BTreeSet<String>,
    /// The fields it uses, directly or through others, sorted.
    pub fields: BTreeSet<String>,
    /// What the chart must compute around the expression: hidden aggregates,
    /// shifted copies and `Fixed` joins (`BI-8` part 2). Empty for part 1.
    pub staged: Staged,
}

/// A hidden aggregate: `sql` computed per group of the chart, under `alias`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiddenAgg {
    /// The column name inside the statement.
    pub alias: String,
    /// The aggregate expression.
    pub sql: String,
}

/// How far back a shifted copy of a hidden aggregate reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShiftKind {
    /// The bucket one grain before.
    Previous,
    /// The same bucket a year before.
    LastYear,
}

/// The value of hidden aggregate `source` one period back, as column `alias`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shift {
    /// The column name inside the statement.
    pub alias: String,
    /// The hidden aggregate it copies.
    pub source: String,
    /// How far back.
    pub kind: ShiftKind,
}

/// `Fixed`: `agg_sql` computed per combination of `columns` over the chart's
/// filtered rows and joined back under `alias`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixedJoin {
    /// The column the join adds.
    pub alias: String,
    /// The source columns it is computed at.
    pub columns: Vec<String>,
    /// The aggregate expression.
    pub agg_sql: String,
}

/// The staged parts of a compiled formula.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Staged {
    /// Hidden aggregates the expression reads.
    pub aggs: Vec<HiddenAgg>,
    /// Shifted copies it reads.
    pub shifts: Vec<Shift>,
    /// `Fixed` joins it reads.
    pub fixed: Vec<FixedJoin>,
}

/// Check `formula` against `scope` and generate its SQL. `own` is the name of
/// the field being saved, so that a reference to itself is a cycle.
///
/// # Errors
///
/// A [`FormulaError`] with the character position of the first problem.
pub fn compile(
    formula: &str,
    scope: &Scope<'_>,
    own: Option<&str>,
) -> Result<Compiled, FormulaError> {
    let tree = parse(formula)?;
    let mut cx = Cx {
        scope,
        stack: own.map(str::to_owned).into_iter().collect(),
        produced: 0,
        reads: BTreeSet::new(),
        fields: BTreeSet::new(),
        staged: Staged::default(),
    };
    let typed = cx.node(&tree)?;
    Ok(Compiled {
        sql: typed.sql,
        ty: typed.ty,
        level: if typed.level == Level::Constant {
            Level::Row
        } else {
            typed.level
        },
        reads: cx.reads,
        fields: cx.fields,
        staged: cx.staged,
    })
}

/// A number as SQL, from its parsed value: whole numbers as integers, the rest
/// through Rust's shortest round-trip form (digits, `.`, `e`, `-`, nothing
/// else).
#[must_use]
pub fn print_number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        #[allow(
            clippy::cast_possible_truncation,
            reason = "v is whole and below 1e15 in magnitude, well inside i64"
        )]
        let whole = v as i64;
        whole.to_string()
    } else {
        format!("{v:?}")
    }
}

mod staged;

struct Typed {
    sql: String,
    ty: FType,
    level: Level,
}

struct Cx<'a> {
    scope: &'a Scope<'a>,
    stack: Vec<String>,
    produced: usize,
    reads: BTreeSet<String>,
    fields: BTreeSet<String>,
    staged: Staged,
}

fn err(node: &Node, message: impl Into<String>) -> FormulaError {
    FormulaError::at(message, node.pos, node.len)
}

fn want(t: &Typed, node: &Node, ty: FType) -> Result<(), FormulaError> {
    if t.ty == ty {
        Ok(())
    } else {
        Err(err(
            node,
            format!("expected {} here, found {}.", ty.word(), t.ty.word()),
        ))
    }
}

fn want_temporal(t: &Typed, node: &Node) -> Result<(), FormulaError> {
    if t.ty.is_temporal() {
        Ok(())
    } else {
        Err(err(
            node,
            format!("expected a date or timestamp here, found {}.", t.ty.word()),
        ))
    }
}

fn want_same(a: &[Typed], nodes: &[Node]) -> Result<FType, FormulaError> {
    let first = a
        .first()
        .ok_or_else(|| FormulaError::at("a value is missing.", 0, 1))?;
    for (t, n) in a.iter().zip(nodes).skip(1) {
        if t.ty != first.ty {
            return Err(err(
                n,
                format!(
                    "this is {} but the value before is {}; they must be the same type.",
                    t.ty.word(),
                    first.ty.word()
                ),
            ));
        }
    }
    Ok(first.ty)
}

/// The level of something built from `args`, or the error for a bare
/// row-level value next to an aggregate (reported at the row-level one).
fn join_levels(args: &[(&Typed, &Node)]) -> Result<Level, FormulaError> {
    let row = args.iter().find(|(t, _)| t.level == Level::Row);
    let has_agg = args.iter().any(|(t, _)| t.level == Level::Aggregate);
    let has_table = args.iter().any(|(t, _)| t.level == Level::Table);
    match (row, has_agg || has_table) {
        (Some((_, n)), true) => Err(err(
            n,
            "this is used on its own next to an aggregate; wrap it in an aggregate such as Sum().",
        )),
        (_, true) if has_table => Ok(Level::Table),
        (_, true) => Ok(Level::Aggregate),
        (Some(_), false) => Ok(Level::Row),
        (None, false) => Ok(Level::Constant),
    }
}

/// A unit written as text: one of the closed list, or an error at the text.
fn unit_of(node: &Node, allowed: &[&'static str]) -> Result<&'static str, FormulaError> {
    let Kind::Str(raw) = &node.kind else {
        return Err(err(
            node,
            format!("write the unit as text, one of {}.", quoted(allowed)),
        ));
    };
    allowed
        .iter()
        .find(|u| u.eq_ignore_ascii_case(raw.trim()))
        .copied()
        .ok_or_else(|| {
            err(
                node,
                format!(
                    "'{}' is not a unit; use one of {}.",
                    clip(raw),
                    quoted(allowed)
                ),
            )
        })
}

fn quoted(words: &[&str]) -> String {
    words
        .iter()
        .map(|w| format!("'{w}'"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A number that must be written out (`2`, `-1`, `0.9`), not computed.
fn literal_number(node: &Node) -> Option<f64> {
    match &node.kind {
        Kind::Num(v) => Some(*v),
        Kind::Neg(inner) => match &inner.kind {
            Kind::Num(v) => Some(-*v),
            _ => None,
        },
        _ => None,
    }
}

/// A whole number on `lo..=hi`, written out.
fn literal_int(node: &Node, lo: i32, hi: i32) -> Result<i32, FormulaError> {
    let bad = || {
        err(
            node,
            format!("write a whole number from {lo} to {hi} here."),
        )
    };
    let v = literal_number(node).ok_or_else(bad)?;
    if v.fract() != 0.0 || v < f64::from(lo) || v > f64::from(hi) {
        return Err(bad());
    }
    #[allow(
        clippy::cast_possible_truncation,
        reason = "v is whole and between two i32 bounds, checked just above"
    )]
    Ok(v as i32)
}

impl Cx<'_> {
    fn node(&mut self, n: &Node) -> Result<Typed, FormulaError> {
        match &n.kind {
            Kind::Num(v) => Ok(Typed {
                sql: print_number(*v),
                ty: FType::Number,
                level: Level::Constant,
            }),
            Kind::Str(s) => Ok(Typed {
                sql: SqlLiteral::from(s.as_str()).to_string(),
                ty: FType::Text,
                level: Level::Constant,
            }),
            Kind::Bool(b) => Ok(Typed {
                sql: b.to_string(),
                ty: FType::Boolean,
                level: Level::Constant,
            }),
            Kind::Name(name) => self.name(name, n),
            Kind::Neg(inner) => {
                let t = self.node(inner)?;
                want(&t, inner, FType::Number)?;
                Ok(Typed {
                    sql: format!("(-{})", t.sql),
                    ..t
                })
            }
            Kind::Not(inner) => {
                let t = self.node(inner)?;
                want(&t, inner, FType::Boolean)?;
                Ok(Typed {
                    sql: format!("(NOT {})", t.sql),
                    ..t
                })
            }
            Kind::Bin(op, l, r) => self.binary(*op, l, r),
            Kind::Call { name, args } => self.call(name, n, args),
        }
    }

    fn name(&mut self, name: &str, node: &Node) -> Result<Typed, FormulaError> {
        if let Some(kind) = self.scope.columns.get(name) {
            let ident = Ident::new(name).map_err(|_| {
                err(
                    node,
                    format!(
                        "the column '{}' cannot be used in a formula (its name has characters other than letters, digits and _).",
                        clip(name)
                    ),
                )
            })?;
            self.reads.insert(name.to_owned());
            return Ok(Typed {
                sql: ident.to_string(),
                ty: FType::of(*kind),
                level: Level::Row,
            });
        }
        if let Some(formula) = self.scope.fields.get(name).cloned() {
            return self.field(name, &formula, node);
        }
        Err(err(
            node,
            format!(
                "'{}' is not a column or a field of this source.",
                clip(name)
            ),
        ))
    }

    /// Expand a reference to another field: its formula is checked and
    /// compiled here, in the same scope, so what it reads is what this
    /// formula reads.
    fn field(&mut self, name: &str, formula: &str, node: &Node) -> Result<Typed, FormulaError> {
        if let Some(at) = self.stack.iter().position(|s| s == name) {
            let mut chain: Vec<&str> = self.stack[at..].iter().map(String::as_str).collect();
            chain.push(name);
            return Err(err(
                node,
                format!(
                    "fields refer to each other in a circle: {}.",
                    chain.join(" > ")
                ),
            ));
        }
        if self.stack.len() > MAX_FIELD_CHAIN {
            return Err(err(
                node,
                format!("fields use each other more than {MAX_FIELD_CHAIN} levels deep."),
            ));
        }
        let tree = parse(formula).map_err(|e| {
            err(
                node,
                format!("the field '{}' has a mistake: {}", clip(name), e.message),
            )
        })?;
        self.stack.push(name.to_owned());
        let inner = self.node(&tree).map_err(|e| {
            err(
                node,
                format!("the field '{}' has a mistake: {}", clip(name), e.message),
            )
        });
        self.stack.pop();
        let inner = inner?;
        self.produced += inner.sql.len();
        if self.produced > MAX_EXPANDED_SQL {
            return Err(err(node, "the fields used here expand into too much."));
        }
        self.fields.insert(name.to_owned());
        Ok(Typed {
            sql: format!("({})", inner.sql),
            ..inner
        })
    }

    fn binary(&mut self, op: BinOp, l: &Node, r: &Node) -> Result<Typed, FormulaError> {
        let a = self.node(l)?;
        let b = self.node(r)?;
        let level = join_levels(&[(&a, l), (&b, r)])?;
        let (sql, ty) = match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => {
                want(&a, l, FType::Number)?;
                want(&b, r, FType::Number)?;
                let sql = match op {
                    BinOp::Add => format!("({} + {})", a.sql, b.sql),
                    BinOp::Sub => format!("({} - {})", a.sql, b.sql),
                    BinOp::Mul => format!("({} * {})", a.sql, b.sql),
                    // Dividing by zero is an empty value, not an error or an
                    // infinity (feature page, decision 5).
                    _ => format!("({} / nullIf({}, 0))", a.sql, b.sql),
                };
                (sql, FType::Number)
            }
            BinOp::And | BinOp::Or => {
                want(&a, l, FType::Boolean)?;
                want(&b, r, FType::Boolean)?;
                let word = if op == BinOp::And { "AND" } else { "OR" };
                (format!("({} {word} {})", a.sql, b.sql), FType::Boolean)
            }
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                if a.ty != b.ty {
                    return Err(err(
                        r,
                        format!(
                            "cannot compare {} with {}; convert one side (ToNumber, ToText, ToDate).",
                            a.ty.word(),
                            b.ty.word()
                        ),
                    ));
                }
                let ordered = !matches!(op, BinOp::Eq | BinOp::Ne);
                if ordered && a.ty == FType::Boolean {
                    return Err(err(l, "true or false can only be compared with = and !=."));
                }
                let sym = match op {
                    BinOp::Eq => "=",
                    BinOp::Ne => "!=",
                    BinOp::Lt => "<",
                    BinOp::Le => "<=",
                    BinOp::Gt => ">",
                    _ => ">=",
                };
                (format!("({} {sym} {})", a.sql, b.sql), FType::Boolean)
            }
        };
        Ok(Typed { sql, ty, level })
    }

    fn call(&mut self, name: &str, node: &Node, nodes: &[Node]) -> Result<Typed, FormulaError> {
        let name_len = name.chars().count();
        let info = catalog::lookup(name).ok_or_else(|| {
            FormulaError::at(
                format!("'{}' is not a function.", clip(name)),
                node.pos,
                name_len,
            )
        })?;
        check_arity(info, nodes, node, name_len)?;
        // BI-8 part 2: these read the chart they are built into and take
        // their arguments in their own way.
        if info.category == "table calculations" || info.category == "period comparison" {
            return self.staged_call(info.name, node, nodes);
        }
        if info.name == "Fixed" {
            return self.fixed(node, nodes);
        }
        let mut args = Vec::with_capacity(nodes.len());
        for n in nodes {
            args.push(self.node(n)?);
        }
        let pairs: Vec<(&Typed, &Node)> = args.iter().zip(nodes).collect();
        let (sql, ty, level) = if info.aggregate {
            if let Some((_, n)) = pairs
                .iter()
                .find(|(t, _)| matches!(t.level, Level::Aggregate | Level::Table))
            {
                return Err(err(n, "an aggregate cannot be inside another aggregate."));
            }
            let (sql, ty) = Self::aggregate(info.name, &args, nodes)?;
            (sql, ty, Level::Aggregate)
        } else {
            let level = join_levels(&pairs)?;
            let (sql, ty) = match info.category {
                "maths" => maths(info.name, &args, nodes)?,
                "text" => self.text(info.name, &args, nodes)?,
                "dates" => self.dates(info.name, &args, nodes)?,
                "conditions" => conditions(info.name, &args, nodes)?,
                _ => self.conversion(info.name, &args, nodes)?,
            };
            // `Today()` and `Now()` take no argument and so are constants of
            // the moment; a call built only from constants is one too.
            (sql, ty, level)
        };
        Ok(Typed { sql, ty, level })
    }
}

fn check_arity(
    info: &FnInfo,
    nodes: &[Node],
    call: &Node,
    name_len: usize,
) -> Result<(), FormulaError> {
    let n = nodes.len();
    if n >= info.min_args && info.max_args.is_none_or(|m| n <= m) {
        return Ok(());
    }
    let wanted = match info.max_args {
        Some(m) if m == info.min_args => format!("{m}"),
        Some(m) => format!("{} to {m}", info.min_args),
        None => format!("at least {}", info.min_args),
    };
    // Too many: point at the first extra argument; too few: at the call.
    let (pos, len) = match info.max_args {
        Some(m) if n > m => (nodes[m].pos, nodes[m].len),
        _ => (call.pos, name_len),
    };
    Err(FormulaError::at(
        format!("{} takes {wanted} argument(s); this has {n}.", info.name),
        pos,
        len,
    ))
}

/// A catalog entry the compiler has no arm for. The test that compiles every
/// entry's example makes this unreachable; it is an error, not a panic, so a
/// catalog edit that forgets the compiler fails closed.
fn unsupported(node: &Node) -> FormulaError {
    err(node, "this function is not available yet.")
}

fn numbers(args: &[Typed], nodes: &[Node]) -> Result<(), FormulaError> {
    for (t, n) in args.iter().zip(nodes) {
        want(t, n, FType::Number)?;
    }
    Ok(())
}

fn joined(args: &[Typed]) -> String {
    args.iter()
        .map(|t| t.sql.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn maths(name: &str, a: &[Typed], n: &[Node]) -> Result<(String, FType), FormulaError> {
    // Round's second argument is a written-out digit count, not a value.
    if name == "Round" {
        want(&a[0], &n[0], FType::Number)?;
        return Ok(match n.get(1) {
            None => (format!("round({})", a[0].sql), FType::Number),
            Some(d) => {
                let digits = literal_int(d, -10, 10)?;
                (format!("round({}, {digits})", a[0].sql), FType::Number)
            }
        });
    }
    numbers(a, n)?;
    let sql = match name {
        "Abs" => format!("abs({})", a[0].sql),
        "Floor" => format!("floor({})", a[0].sql),
        "Ceil" => format!("ceil({})", a[0].sql),
        "Exp" => format!("exp({})", a[0].sql),
        "Sqrt" => format!("if({0} >= 0, sqrt({0}), NULL)", a[0].sql),
        "Log" => format!("if({0} > 0, log({0}), NULL)", a[0].sql),
        "Power" => format!("pow({}, {})", a[0].sql, a[1].sql),
        "Mod" => format!("modulo({}, nullIf({}, 0))", a[0].sql, a[1].sql),
        "Greatest" => format!("greatest({})", joined(a)),
        "Least" => format!("least({})", joined(a)),
        _ => return Err(unsupported(&n[0])),
    };
    Ok((sql, FType::Number))
}

impl Cx<'_> {
    /// `value` as text for `Concat` and `ToText`.
    fn as_text(&self, t: &Typed) -> String {
        match t.ty {
            FType::Text => t.sql.clone(),
            FType::DateTime => format!("toString({})", self.scope.time.in_zone_expr(&t.sql)),
            FType::Boolean => format!("if({}, 'true', 'false')", t.sql),
            FType::Number | FType::Date => format!("toString({})", t.sql),
        }
    }

    fn text(&self, name: &str, a: &[Typed], n: &[Node]) -> Result<(String, FType), FormulaError> {
        if name == "Concat" {
            let parts: Vec<String> = a
                .iter()
                .map(|t| format!("ifNull({}, '')", self.as_text(t)))
                .collect();
            return Ok((format!("concat({})", parts.join(", ")), FType::Text));
        }
        want(&a[0], &n[0], FType::Text)?;
        let s = &a[0].sql;
        let count = |i: usize| -> Result<String, FormulaError> {
            want(&a[i], &n[i], FType::Number)?;
            Ok(format!("greatest(toInt64({}), 0)", a[i].sql))
        };
        let (sql, ty) = match name {
            "Upper" => (format!("upperUTF8({s})"), FType::Text),
            "Lower" => (format!("lowerUTF8({s})"), FType::Text),
            "Trim" => (format!("trimBoth({s})"), FType::Text),
            "Length" => (format!("lengthUTF8({s})"), FType::Number),
            "Substring" => {
                want(&a[1], &n[1], FType::Number)?;
                // The engine refuses a start of 0 or less ("indices in strings
                // are 1-based", measured on 26.8); a start below 1 is 1.
                let start = format!("greatest(toInt64({}), 1)", a[1].sql);
                let sql = if a.len() == 3 {
                    format!("substringUTF8({s}, {start}, {})", count(2)?)
                } else {
                    format!("substringUTF8({s}, {start})")
                };
                (sql, FType::Text)
            }
            "Left" => (format!("substringUTF8({s}, 1, {})", count(1)?), FType::Text),
            // A start of 0 is refused by the engine as well, so "no
            // characters" is spelled out.
            "Right" => {
                want(&a[1], &n[1], FType::Number)?;
                let k = format!("toInt64({})", a[1].sql);
                (
                    format!("if({k} > 0, substringUTF8({s}, -{k}), '')"),
                    FType::Text,
                )
            }
            _ => {
                for i in 1..a.len() {
                    want(&a[i], &n[i], FType::Text)?;
                }
                let t = &a[1].sql;
                match name {
                    "Replace" => (format!("replaceAll({s}, {t}, {})", a[2].sql), FType::Text),
                    "Contains" => (format!("(positionUTF8({s}, {t}) > 0)"), FType::Boolean),
                    "StartsWith" => (format!("startsWith({s}, {t})"), FType::Boolean),
                    "EndsWith" => (format!("endsWith({s}, {t})"), FType::Boolean),
                    _ => return Err(unsupported(&n[0])),
                }
            }
        };
        Ok((sql, ty))
    }

    fn dates(&self, name: &str, a: &[Typed], n: &[Node]) -> Result<(String, FType), FormulaError> {
        let time = self.scope.time;
        match name {
            "Today" => return Ok((time.today_expr(), FType::Date)),
            "Now" => return Ok((time.now_expr(), FType::DateTime)),
            "DateTrunc" | "DateAdd" | "DateDiff" => return self.date_unit_fn(name, a, n),
            _ => {}
        }
        want_temporal(&a[0], &n[0])?;
        if name == "Hour" && a[0].ty != FType::DateTime {
            return Err(err(&n[0], "Hour needs a timestamp; this date has no time."));
        }
        let x = if a[0].ty == FType::DateTime {
            time.in_zone_expr(&a[0].sql)
        } else {
            a[0].sql.clone()
        };
        let f = match name {
            "Year" => "toYear",
            "Month" => "toMonth",
            "Day" => "toDayOfMonth",
            "Hour" => "toHour",
            "Weekday" => "toDayOfWeek",
            _ => return Err(unsupported(&n[0])),
        };
        Ok((format!("{f}({x})"), FType::Number))
    }

    /// `DateTrunc`, `DateAdd` and `DateDiff`: a unit word, then dates.
    fn date_unit_fn(
        &self,
        name: &str,
        a: &[Typed],
        n: &[Node],
    ) -> Result<(String, FType), FormulaError> {
        let time = self.scope.time;
        let unit = unit_of(&n[0], &UNITS)?;
        let needs_time = matches!(unit, "minute" | "hour");
        let timestamp_only = |t: &Typed, node: &Node| -> Result<(), FormulaError> {
            if needs_time && t.ty != FType::DateTime {
                Err(err(
                    node,
                    format!("'{unit}' needs a timestamp; this date has no time."),
                ))
            } else {
                Ok(())
            }
        };
        match name {
            "DateTrunc" => {
                want_temporal(&a[1], &n[1])?;
                timestamp_only(&a[1], &n[1])?;
                let grain = Grain::parse(unit).ok_or_else(|| err(&n[0], "not a unit."))?;
                let kind = if a[1].ty == FType::DateTime {
                    ColumnKind::DateTime
                } else {
                    ColumnKind::Date
                };
                let sql = grain
                    .bucket_expr_over(&a[1].sql, kind, time)
                    .ok_or_else(|| err(&n[1], "this unit does not fit this value."))?;
                let ty = if needs_time {
                    FType::DateTime
                } else {
                    FType::Date
                };
                Ok((sql, ty))
            }
            "DateAdd" => {
                want(&a[1], &n[1], FType::Number)?;
                want_temporal(&a[2], &n[2])?;
                timestamp_only(&a[2], &n[2])?;
                let f = match unit {
                    "minute" => "addMinutes",
                    "hour" => "addHours",
                    "day" => "addDays",
                    "week" => "addWeeks",
                    "month" => "addMonths",
                    "quarter" => "addQuarters",
                    "year" => "addYears",
                    _ => return Err(unsupported(&n[0])),
                };
                Ok((format!("{f}({}, toInt64({}))", a[2].sql, a[1].sql), a[2].ty))
            }
            _ => {
                want_temporal(&a[1], &n[1])?;
                want_temporal(&a[2], &n[2])?;
                timestamp_only(&a[1], &n[1])?;
                timestamp_only(&a[2], &n[2])?;
                // A date and a timestamp compare as days; two timestamps in
                // the report zone.
                let both_time = a[1].ty == FType::DateTime && a[2].ty == FType::DateTime;
                let side = |t: &Typed| {
                    if t.ty == FType::DateTime {
                        let zoned = time.in_zone_expr(&t.sql);
                        if both_time {
                            zoned
                        } else {
                            format!("toDate32({zoned})")
                        }
                    } else {
                        t.sql.clone()
                    }
                };
                Ok((
                    format!("dateDiff('{unit}', {}, {})", side(&a[1]), side(&a[2])),
                    FType::Number,
                ))
            }
        }
    }

    fn conversion(
        &self,
        name: &str,
        a: &[Typed],
        n: &[Node],
    ) -> Result<(String, FType), FormulaError> {
        let x = &a[0];
        match name {
            "ToNumber" => match x.ty {
                FType::Text => Ok((format!("toFloat64OrNull({})", x.sql), FType::Number)),
                FType::Number => Ok((x.sql.clone(), FType::Number)),
                _ => Err(err(&n[0], "ToNumber reads text or a number.")),
            },
            "ToText" => Ok((self.as_text(x), FType::Text)),
            "ToDate" => match x.ty {
                FType::Text => Ok((format!("toDate32OrNull({})", x.sql), FType::Date)),
                FType::Date => Ok((x.sql.clone(), FType::Date)),
                FType::DateTime => Ok((
                    format!("toDate32({})", self.scope.time.in_zone_expr(&x.sql)),
                    FType::Date,
                )),
                _ => Err(err(&n[0], "ToDate reads text, a date or a timestamp.")),
            },
            _ => Err(unsupported(&n[0])),
        }
    }

    fn aggregate(name: &str, a: &[Typed], n: &[Node]) -> Result<(String, FType), FormulaError> {
        let cond = |i: usize| want(&a[i], &n[i], FType::Boolean);
        match name {
            "Count" => Ok((
                a.first()
                    .map_or_else(|| "count()".to_owned(), |t| format!("count({})", t.sql)),
                FType::Number,
            )),
            "CountDistinct" => Ok((format!("uniqExact({})", a[0].sql), FType::Number)),
            "CountIf" => {
                cond(0)?;
                Ok((format!("countIf({})", a[0].sql), FType::Number))
            }
            "Min" | "Max" => {
                if a[0].ty == FType::Boolean {
                    return Err(err(&n[0], "Min and Max read numbers, text or dates."));
                }
                Ok((
                    format!("{}({})", name.to_ascii_lowercase(), a[0].sql),
                    a[0].ty,
                ))
            }
            _ => {
                want(&a[0], &n[0], FType::Number)?;
                let x = &a[0].sql;
                let sql = match name {
                    "Sum" => format!("sum({x})"),
                    "Avg" => format!("avg({x})"),
                    "Median" => format!("medianExact({x})"),
                    "StdDev" => format!("stddevSamp({x})"),
                    "Percentile" => {
                        let p = literal_number(&n[1])
                            .filter(|p| (0.0..=1.0).contains(p))
                            .ok_or_else(|| {
                                err(
                                    &n[1],
                                    "write the share as a number from 0 to 1, such as 0.9.",
                                )
                            })?;
                        format!("quantileExact({})({x})", print_number(p))
                    }
                    "SumIf" | "AvgIf" => {
                        cond(1)?;
                        let f = if name == "SumIf" { "sumIf" } else { "avgIf" };
                        format!("{f}({x}, {})", a[1].sql)
                    }
                    _ => return Err(unsupported(&n[0])),
                };
                Ok((sql, FType::Number))
            }
        }
    }
}

fn conditions(name: &str, a: &[Typed], n: &[Node]) -> Result<(String, FType), FormulaError> {
    match name {
        "If" => {
            want(&a[0], &n[0], FType::Boolean)?;
            let ty = want_same(&a[1..], &n[1..])?;
            Ok((format!("if({})", joined(a)), ty))
        }
        "Case" => {
            if a.len().is_multiple_of(2) {
                return Err(err(
                    &n[n.len() - 1],
                    "Case takes pairs of test and value, then a last value for everything else.",
                ));
            }
            for i in (0..a.len() - 1).step_by(2) {
                want(&a[i], &n[i], FType::Boolean)?;
            }
            let values: Vec<&Typed> = (1..a.len())
                .step_by(2)
                .map(|i| &a[i])
                .chain(a.last())
                .collect();
            let value_nodes: Vec<&Node> = (1..a.len())
                .step_by(2)
                .map(|i| &n[i])
                .chain(n.last())
                .collect();
            let first = values[0].ty;
            for (t, node) in values.iter().zip(&value_nodes).skip(1) {
                if t.ty != first {
                    return Err(err(
                        node,
                        format!(
                            "this is {} but the first value is {}; they must be the same type.",
                            t.ty.word(),
                            first.word()
                        ),
                    ));
                }
            }
            Ok((format!("multiIf({})", joined(a)), first))
        }
        "Coalesce" => {
            let ty = want_same(a, n)?;
            Ok((format!("coalesce({})", joined(a)), ty))
        }
        "IsNull" => Ok((format!("({} IS NULL)", a[0].sql), FType::Boolean)),
        "Between" => {
            let ty = want_same(a, n)?;
            if ty == FType::Boolean {
                return Err(err(&n[0], "Between reads numbers, text or dates."));
            }
            Ok((
                format!("({} BETWEEN {} AND {})", a[0].sql, a[1].sql, a[2].sql),
                FType::Boolean,
            ))
        }
        _ => Err(unsupported(&n[0])),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::filters::ColumnKind;

    fn sample_columns() -> RelationColumns {
        [
            ("amount", ColumnKind::Number),
            ("qty", ColumnKind::Number),
            ("label", ColumnKind::Text),
            ("day", ColumnKind::Date),
            ("ts", ColumnKind::DateTime),
            ("Net Sales", ColumnKind::Number),
            ("a'b", ColumnKind::Number),
        ]
        .into_iter()
        .map(|(n, k)| (n.to_owned(), k))
        .collect()
    }

    fn run(formula: &str) -> Result<Compiled, FormulaError> {
        let cols = sample_columns();
        let time = TimeContext::default();
        compile(formula, &Scope::new(&cols, &time), None)
    }

    fn ok(formula: &str) -> Compiled {
        run(formula).unwrap_or_else(|e| panic!("{formula}: {e}"))
    }

    fn err_at(formula: &str) -> FormulaError {
        run(formula).expect_err(formula)
    }

    #[test]
    fn every_catalog_example_compiles_with_the_level_the_catalog_says() {
        for f in catalog::CATALOG {
            let c = ok(f.example);
            let expected = if f.table {
                Level::Table
            } else if f.aggregate {
                Level::Aggregate
            } else {
                Level::Row
            };
            assert_eq!(c.level, expected, "{}: {}", f.name, f.example);
        }
    }

    #[test]
    fn arithmetic_compiles_to_parenthesised_sql_and_division_guards_zero() {
        let c = ok("[amount] - qty * 2");
        assert_eq!(c.sql, "(amount - (qty * 2))");
        assert_eq!((c.ty, c.level), (FType::Number, Level::Row));
        assert_eq!(
            c.reads.iter().cloned().collect::<Vec<_>>(),
            ["amount", "qty"]
        );
        assert_eq!(ok("amount / qty").sql, "(amount / nullIf(qty, 0))");
        assert_eq!(ok("-(amount)").sql, "(-amount)");
        assert_eq!(ok("- -amount").sql, "(-(-amount))");
        assert_eq!(ok("amount - -3").sql, "(amount - (-3))");
    }

    #[test]
    fn numbers_are_reprinted_from_their_value() {
        assert_eq!(ok("1.50").sql, "1.5");
        assert_eq!(ok("007").sql, "7");
        assert_eq!(ok("1e3").sql, "1000");
        assert_eq!(ok("2.5e-7").sql, "2.5e-7");
        assert_eq!(print_number(0.1), "0.1");
        assert_eq!(print_number(123_456_789_012_345.0), "123456789012345");
    }

    #[test]
    fn text_goes_through_the_literal_helper() {
        assert_eq!(ok("'it''s'").sql, "'it''s'");
        assert_eq!(ok(r"'a\b'").sql, r"'a\\b'");
        assert_eq!(ok("Upper(\"x\")").sql, "upperUTF8('x')");
    }

    #[test]
    fn a_level_mixture_is_refused_at_the_bare_column() {
        let e = err_at("Sum(amount) / qty");
        assert_eq!((e.position, e.length), (14, 3), "{e:?}");
        let e = err_at("qty + Sum(amount)");
        assert_eq!(e.position, 0, "{e:?}");
        let e = err_at("If(amount > 1, Sum(qty), 0)");
        assert_eq!(e.position, 3, "the test is row-level next to an aggregate");
        assert_eq!(ok("Sum(amount) / Count()").level, Level::Aggregate);
        assert_eq!(ok("Sum(amount) / 2").level, Level::Aggregate);
        assert_eq!(ok("Sum(amount * qty)").level, Level::Aggregate);
        assert_eq!(ok("amount * 2").level, Level::Row);
        assert_eq!(
            ok("1 + 2").level,
            Level::Row,
            "a constant stores as row-level"
        );
    }

    #[test]
    fn an_aggregate_inside_an_aggregate_is_refused_at_the_inner_one() {
        let e = err_at("Sum(Avg(amount))");
        assert_eq!(e.position, 4, "{e:?}");
        assert!(e.message.contains("inside another aggregate"));
        assert!(run("SumIf(amount, Sum(qty) > 1)").is_err());
    }

    #[test]
    fn each_error_class_carries_its_position() {
        // (formula, position, a word of the message)
        let cases = [
            ("amount +", 7, "ends"),
            ("Sleep(1)", 0, "not a function"),
            ("1 + Sleep(1)", 4, "not a function"),
            ("nope + 1", 0, "not a column"),
            ("[Net Sales] + 1", 0, "cannot be used"),
            ("amount + 'x'", 9, "expected a number"),
            ("label + 1", 0, "expected a number"),
            ("amount = label", 9, "cannot compare"),
            ("If(amount, 1, 2)", 3, "true or false"),
            ("If(amount > 1, 1, 'x')", 18, "same type"),
            ("Sum(amount, qty)", 12, "takes 1"),
            ("Round()", 0, "takes 1 to 2"),
            ("Round(amount, qty)", 14, "whole number"),
            ("Round(amount, 99)", 14, "whole number"),
            ("Percentile(amount, 1.5)", 19, "from 0 to 1"),
            ("DateTrunc('fortnight', day)", 10, "not a unit"),
            ("DateTrunc(amount, day)", 10, "unit as text"),
            ("DateTrunc('hour', day)", 18, "needs a timestamp"),
            ("Hour(day)", 5, "needs a timestamp"),
            ("Year(amount)", 5, "date or timestamp"),
            ("Case(amount > 1, 'a', amount > 0, 'b')", 34, "pairs"),
            ("not amount", 4, "true or false"),
            ("Between(amount, 1, 'x')", 19, "same type"),
            ("ToNumber(day)", 9, "text or a number"),
            ("Min(amount > 1)", 4, "numbers, text or dates"),
            ("amount < qty < 3", 13, "do not chain"),
        ];
        for (formula, position, word) in cases {
            let e = err_at(formula);
            assert_eq!(e.position, position, "{formula}: {e:?}");
            assert!(e.message.contains(word), "{formula}: {e:?}");
        }
    }

    #[test]
    fn dates_compile_in_the_report_zone() {
        let t = TimeContext::default();
        assert_eq!(ok("Year(ts)").sql, "toYear(toTimeZone(ts, 'Asia/Jakarta'))");
        assert_eq!(ok("Month(day)").sql, "toMonth(day)");
        assert_eq!(ok("Today()").sql, t.today_expr());
        assert_eq!(ok("Now()").sql, t.now_expr());
        assert_eq!(ok("Today()").ty, FType::Date);
        assert_eq!(
            ok("DateTrunc('month', ts)").sql,
            Grain::Month
                .bucket_expr_over("ts", ColumnKind::DateTime, &t)
                .unwrap()
        );
        assert_eq!(ok("DateTrunc('hour', ts)").ty, FType::DateTime);
        assert_eq!(ok("DateTrunc('Month', day)").ty, FType::Date);
        assert_eq!(
            ok("DateAdd('month', 1, day)").sql,
            "addMonths(day, toInt64(1))"
        );
        assert_eq!(ok("DateAdd('hour', 1, ts)").ty, FType::DateTime);
        assert_eq!(
            ok("DateDiff('day', day, ts)").sql,
            "dateDiff('day', day, toDate32(toTimeZone(ts, 'Asia/Jakarta')))"
        );
        assert_eq!(
            ok("DateDiff('hour', ts, Now())").sql,
            "dateDiff('hour', toTimeZone(ts, 'Asia/Jakarta'), toTimeZone(now('Asia/Jakarta'), 'Asia/Jakarta'))"
        );
    }

    #[test]
    fn today_and_the_zone_follow_the_deployment_settings() {
        let cols = sample_columns();
        let time = TimeContext::new("America/New_York", crate::grain::WeekStart::Sunday).unwrap();
        let c = compile("Today()", &Scope::new(&cols, &time), None).unwrap();
        assert!(c.sql.contains("America/New_York"), "{}", c.sql);
        let c = compile("DateTrunc('week', day)", &Scope::new(&cols, &time), None).unwrap();
        assert!(
            c.sql.contains("toIntervalDay(1)"),
            "Sunday weeks: {}",
            c.sql
        );
    }

    #[test]
    fn conditions_and_conversions_have_the_documented_types() {
        assert_eq!(ok("If(amount > 1, 'a', 'b')").ty, FType::Text);
        assert_eq!(
            ok("Case(amount > 1, 1, amount > 0, 2, 3)").sql,
            "multiIf((amount > 1), 1, (amount > 0), 2, 3)"
        );
        assert_eq!(ok("Coalesce(label, 'x')").sql, "coalesce(label, 'x')");
        assert_eq!(ok("IsNull(label)").ty, FType::Boolean);
        assert_eq!(
            ok("Between(day, ToDate('2026-01-01'), Today())").ty,
            FType::Boolean
        );
        assert_eq!(ok("ToNumber(label)").sql, "toFloat64OrNull(label)");
        assert_eq!(
            ok("ToText(ts)").sql,
            "toString(toTimeZone(ts, 'Asia/Jakarta'))"
        );
        assert_eq!(ok("ToText(amount > 1)").ty, FType::Text);
        assert_eq!(ok("ToDate(ts)").ty, FType::Date);
        assert_eq!(ok("true and not false").ty, FType::Boolean);
    }

    #[test]
    fn aggregates_compile_to_the_engines_functions() {
        assert_eq!(ok("Count()").sql, "count()");
        assert_eq!(ok("CountDistinct(label)").sql, "uniqExact(label)");
        assert_eq!(ok("Median(amount)").sql, "medianExact(amount)");
        assert_eq!(
            ok("Percentile(amount, 0.9)").sql,
            "quantileExact(0.9)(amount)"
        );
        assert_eq!(ok("Percentile(amount, 1)").sql, "quantileExact(1)(amount)");
        assert_eq!(ok("CountIf(amount > 1)").sql, "countIf((amount > 1))");
        assert_eq!(ok("Max(day)").ty, FType::Date);
        assert_eq!(ok("Min(label)").ty, FType::Text);
    }

    #[test]
    fn fields_are_expanded_in_place_and_report_what_they_use() {
        let cols = sample_columns();
        let time = TimeContext::default();
        let scope = Scope::new(&cols, &time)
            .with_field("profit", "amount - qty")
            .with_field("margin", "profit / amount");
        let c = compile("margin * 100", &scope, None).unwrap();
        assert_eq!(c.sql, "(((((amount - qty)) / nullIf(amount, 0))) * 100)");
        assert_eq!(
            c.fields.iter().cloned().collect::<Vec<_>>(),
            ["margin", "profit"]
        );
        assert_eq!(
            c.reads.iter().cloned().collect::<Vec<_>>(),
            ["amount", "qty"]
        );
    }

    #[test]
    fn a_field_that_uses_itself_or_a_circle_is_refused() {
        let cols = sample_columns();
        let time = TimeContext::default();
        let scope = Scope::new(&cols, &time)
            .with_field("a", "b + 1")
            .with_field("b", "a + 1");
        let e = compile("a", &scope, None).unwrap_err();
        assert!(e.message.contains("circle"), "{e:?}");
        let e = compile(
            "c + 1",
            &Scope::new(&cols, &time).with_field("c", "1"),
            Some("c"),
        )
        .unwrap_err();
        assert!(e.message.contains("circle") && e.position == 0, "{e:?}");
    }

    #[test]
    fn a_field_chain_deeper_than_eight_is_refused_and_eight_is_allowed() {
        let cols = sample_columns();
        let time = TimeContext::default();
        let build = |n: usize| {
            let mut scope = Scope::new(&cols, &time).with_field("f0", "amount");
            for i in 1..n {
                scope = scope.with_field(&format!("f{i}"), &format!("f{} + 1", i - 1));
            }
            scope
        };
        assert!(compile("f7", &build(8), None).is_ok());
        let e = compile("f9", &build(10), None).unwrap_err();
        assert!(e.message.contains("levels deep"), "{e:?}");
    }

    #[test]
    fn a_mistake_inside_a_used_field_is_reported_at_the_reference() {
        let cols = sample_columns();
        let time = TimeContext::default();
        let scope = Scope::new(&cols, &time).with_field("bad", "amount +");
        let e = compile("1 + bad", &scope, None).unwrap_err();
        assert_eq!(e.position, 4, "{e:?}");
        assert!(e.message.contains("'bad' has a mistake"), "{e:?}");
    }

    #[test]
    fn references_cannot_blow_a_short_formula_up_into_a_huge_statement() {
        let cols = sample_columns();
        let time = TimeContext::default();
        // Each field doubles the one before it; seven levels of that is
        // already past the cap if the sizes were not counted.
        let mut scope = Scope::new(&cols, &time)
            .with_field("g0", &format!("{}amount", "amount + ".repeat(100)));
        for i in 1..8 {
            let prev = format!("g{}", i - 1);
            scope = scope.with_field(
                &format!("g{i}"),
                &format!("{}{prev}", format!("{prev} + ").repeat(30)),
            );
        }
        let e = compile("g7", &scope, None).unwrap_err();
        assert!(e.message.contains("too much"), "{e:?}");
    }

    // ── injection ────────────────────────────────────────────────────────

    /// Walk `sql`: outside literals only a closed set of characters may
    /// appear (no `;`, backtick, double quote, backslash, comment markers,
    /// non-ASCII); every literal is closed and is returned decoded with the
    /// escape rules of `SqlLiteral`.
    fn audit_sql(sql: &str) -> Vec<String> {
        let chars: Vec<char> = sql.chars().collect();
        let mut literals = Vec::new();
        let mut i = 0;
        let mut outside = String::new();
        while i < chars.len() {
            let c = chars[i];
            if c == '\'' {
                let mut text = String::new();
                i += 1;
                loop {
                    let Some(&d) = chars.get(i) else {
                        panic!("unterminated literal in {sql}");
                    };
                    match d {
                        '\'' if chars.get(i + 1) == Some(&'\'') => {
                            text.push('\'');
                            i += 2;
                        }
                        '\'' => {
                            i += 1;
                            break;
                        }
                        '\\' => {
                            assert_eq!(chars.get(i + 1), Some(&'\\'), "lone backslash in {sql}");
                            text.push('\\');
                            i += 2;
                        }
                        other => {
                            text.push(other);
                            i += 1;
                        }
                    }
                }
                literals.push(text);
                outside.push('\'');
                continue;
            }
            assert!(
                c.is_ascii_alphanumeric() || " _(),.+-*/<>=!".contains(c),
                "character {c:?} outside a literal in {sql}"
            );
            outside.push(c);
            i += 1;
        }
        assert!(
            !outside.contains("--") && !outside.contains("/*"),
            "comment marker in {sql}"
        );
        literals
    }

    /// A small deterministic generator (xorshift), so a failure repeats.
    struct Rng(u64);
    #[allow(
        clippy::cast_possible_truncation,
        reason = "test generator: every value is reduced modulo a slice length first"
    )]
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
            &items[(self.next() % items.len() as u64) as usize]
        }
    }

    const NASTY_CHARS: &[char] = &[
        '\'',
        '"',
        '\\',
        '`',
        ';',
        '-',
        '/',
        '*',
        '[',
        ']',
        '(',
        ')',
        ',',
        ' ',
        '\n',
        '\t',
        '%',
        '_',
        '$',
        '{',
        '}',
        '#',
        '|',
        ':',
        '@',
        '\u{e9}',
        '\u{65e5}',
        '\u{1f600}',
        'a',
        'Z',
        '0',
        '9',
        '.',
        '=',
        '<',
        '>',
        '!',
    ];

    fn nasty(rng: &mut Rng, max: usize) -> String {
        let len = 1 + rng.pick(&(0..max).collect::<Vec<_>>());
        (0..len).map(|_| *rng.pick(NASTY_CHARS)).collect()
    }

    #[test]
    fn generated_hostile_text_never_reaches_sql_outside_an_escaped_literal() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let mut compiled = 0_u32;
        let mut refused = 0_u32;
        for _ in 0..20_000 {
            let text = nasty(&mut rng, 40);
            let quote = *rng.pick(&['\'', '"']);
            let escaped = text.replace(quote, &format!("{quote}{quote}"));
            // 1. hostile text as a quoted string: it must come out decoded to
            //    exactly the same text, inside one literal.
            let formula = format!("Concat({quote}{escaped}{quote}, label)");
            let c = run(&formula).unwrap_or_else(|e| panic!("{formula:?}: {e}"));
            let literals = audit_sql(&c.sql);
            assert_eq!(literals[0], text, "{formula:?} -> {}", c.sql);
            compiled += 1;
            // 2. hostile text as a column name or as free source text: either
            //    a refusal with a position, or SQL that passes the audit.
            for source in [
                format!("[{text}] + 1"),
                text.clone(),
                format!("amount {text} 1"),
                format!("If({text}, 1, 2)"),
                format!("DateTrunc('{}', day)", text.replace('\'', "''")),
            ] {
                match run(&source) {
                    Ok(c) => {
                        audit_sql(&c.sql);
                        compiled += 1;
                    }
                    Err(e) => {
                        assert!(e.position <= source.chars().count(), "{source:?}: {e:?}");
                        refused += 1;
                    }
                }
            }
        }
        assert!(
            compiled > 20_000 && refused > 10_000,
            "{compiled} compiled, {refused} refused"
        );
    }

    #[test]
    fn a_column_name_with_a_quote_or_a_space_is_refused_not_written() {
        let e = err_at("[a'b] + 1");
        assert!(e.message.contains("cannot be used"), "{e:?}");
        let e = err_at("[Net Sales]");
        assert!(e.message.contains("cannot be used"), "{e:?}");
        // A name that is not a column of the source is refused whatever its
        // shape; it is never looked up as a function or written as it is.
        assert!(run("[drop table x]").is_err());
        assert!(run("amount; DROP TABLE x").is_err());
        assert!(run("amount -- c").is_err());
        assert!(run("amount /* c */").is_err());
    }

    #[test]
    fn units_are_written_from_the_closed_list_never_from_the_text() {
        let c = ok("DateAdd('MONTH', 1, day)");
        assert!(c.sql.starts_with("addMonths("), "{}", c.sql);
        assert!(run("DateAdd('month); DROP', 1, day)").is_err());
        assert!(
            run("DateAdd(' day ', 1, day)").is_ok(),
            "trimmed, then matched exactly"
        );
    }

    #[test]
    fn the_limits_are_enforced_with_a_position() {
        let long = format!("{}1", "1+".repeat(1000));
        assert_eq!(long.chars().count(), 2001);
        assert_eq!(err_at(&long).position, 2000);
        let deep = format!("{}1{}", "(".repeat(33), ")".repeat(33));
        assert!(err_at(&deep).message.contains("32"));
        assert!(run(&format!("{}1{}", "(".repeat(31), ")".repeat(31))).is_ok());
    }

    // ── BI-8 part 2: table calculations, period comparisons, Fixed ───────

    fn in_chart(formula: &str, ctx: ChartCtx) -> Result<Compiled, FormulaError> {
        let cols = sample_columns();
        let time = TimeContext::default();
        compile(
            formula,
            &Scope::new(&cols, &time).with_chart(ctx),
            Some("f"),
        )
    }

    fn by_label() -> ChartCtx {
        ChartCtx {
            dimension: Some("label".to_owned()),
            breakdown: None,
            order_key: "label".to_owned(),
            grain: None,
        }
    }

    fn by_month() -> ChartCtx {
        ChartCtx {
            dimension: Some("day".to_owned()),
            breakdown: Some("label".to_owned()),
            order_key: "day".to_owned(),
            grain: Some(Grain::Month),
        }
    }

    #[test]
    fn a_table_calculation_is_a_window_over_hidden_aggregates_in_the_charts_order() {
        let c = in_chart("RunningTotal(Sum(amount))", by_label()).unwrap();
        assert_eq!(c.level, Level::Table);
        let a = &c.staged.aggs[0].alias;
        assert!(a.starts_with("__a_"), "{a}");
        assert_eq!(c.staged.aggs[0].sql, "sum(amount)");
        assert_eq!(
            c.sql,
            format!(
                "sum({a}) OVER (ORDER BY label ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW)"
            )
        );
        // With a breakdown the window is partitioned by it; PercentOfTotal is not.
        let c = in_chart("RunningTotal(Sum(amount))", by_month()).unwrap();
        assert!(
            c.sql.contains("OVER (PARTITION BY label ORDER BY day ROWS"),
            "{}",
            c.sql
        );
        let c = in_chart("PercentOfTotal(Sum(amount))", by_month()).unwrap();
        assert!(
            c.sql.contains("sum(") && c.sql.contains(") OVER ()"),
            "{}",
            c.sql
        );
        assert!(!c.sql.contains("PARTITION"), "{}", c.sql);
        let c = in_chart("Rank(Sum(amount))", by_month()).unwrap();
        assert!(
            c.sql
                .starts_with("rank() OVER (PARTITION BY label ORDER BY __a_"),
            "{}",
            c.sql
        );
        let c = in_chart("MovingAverage(Sum(amount), 3)", by_label()).unwrap();
        assert!(
            c.sql.contains("ROWS BETWEEN 2 PRECEDING AND CURRENT ROW"),
            "{}",
            c.sql
        );
        let c = in_chart("Offset(Sum(amount), -2)", by_label()).unwrap();
        assert!(c.sql.starts_with("leadInFrame(toNullable("), "{}", c.sql);
        let c = in_chart("RunningCount()", by_label()).unwrap();
        assert_eq!(c.staged.aggs[0].sql, "count()");
        let c = in_chart("RunningCount(label)", by_label()).unwrap();
        assert_eq!(c.staged.aggs[0].sql, "count(label)");
    }

    #[test]
    fn the_same_aggregate_used_twice_is_one_hidden_column() {
        let c = in_chart(
            "RunningTotal(Sum(amount)) / PercentOfTotal(Sum(amount))",
            by_label(),
        )
        .unwrap();
        assert_eq!(c.staged.aggs.len(), 1);
        assert_eq!(c.level, Level::Table);
        // An aggregate beside a table calculation is allowed (both are
        // computed from the grouped result); a bare column is not.
        assert_eq!(
            in_chart("Sum(amount) - RunningTotal(Sum(amount))", by_label())
                .unwrap()
                .level,
            Level::Table
        );
        assert!(in_chart("amount - RunningTotal(Sum(amount))", by_label()).is_err());
    }

    #[test]
    fn each_misuse_of_a_table_calculation_is_refused_where_it_is() {
        let at = |f: &str, ctx: ChartCtx| in_chart(f, ctx).expect_err(f);
        let e = at("RunningTotal(amount)", by_label());
        assert_eq!((e.position, e.length), (13, 6), "{e:?}");
        assert!(e.message.contains("takes an aggregate"));
        let e = at("RunningTotal(RunningTotal(Sum(amount)))", by_label());
        assert_eq!(e.position, 13, "{e:?}");
        assert!(e.message.contains("inside another"));
        let e = at("MovingAverage(Sum(amount), 0)", by_label());
        assert_eq!(e.position, 27, "{e:?}");
        let e = at("Offset(Sum(amount), 0)", by_label());
        assert_eq!(e.position, 20, "{e:?}");
        let e = at("Sum(RunningTotal(Sum(amount)))", by_label());
        assert!(e.message.contains("inside another aggregate"), "{e:?}");
        let e = at("RunningTotal(Sum(label))", by_label());
        assert!(e.message.contains("expected a number"), "{e:?}");
        let no_dimension = ChartCtx {
            dimension: None,
            ..by_label()
        };
        let e = at("Rank(Sum(amount))", no_dimension);
        assert_eq!(e.position, 0, "{e:?}");
        assert!(e.message.contains("needs a chart with a dimension"));
    }

    #[test]
    fn a_period_comparison_needs_a_date_grain_and_names_what_it_reads() {
        let c = in_chart("PreviousPeriod(Sum(amount))", by_month()).unwrap();
        assert_eq!(c.level, Level::Table);
        assert_eq!(c.staged.shifts.len(), 1);
        assert_eq!(c.sql, c.staged.shifts[0].alias);
        assert_eq!(c.staged.shifts[0].kind, ShiftKind::Previous);
        let y = in_chart("SamePeriodLastYear(Sum(amount))", by_month()).unwrap();
        assert_eq!(y.staged.shifts[0].kind, ShiftKind::LastYear);
        assert_ne!(y.staged.shifts[0].alias, c.staged.shifts[0].alias);
        let e = in_chart("PreviousPeriod(Sum(amount))", by_label()).unwrap_err();
        assert!(e.message.contains("Group by"), "{e:?}");
        let part = ChartCtx {
            grain: Some(Grain::MonthOfYear),
            ..by_month()
        };
        assert!(
            in_chart("PreviousPeriod(Sum(amount))", part)
                .unwrap_err()
                .message
                .contains("not a part")
        );
        let hourly = ChartCtx {
            grain: Some(Grain::Hour),
            ..by_month()
        };
        assert!(in_chart("PreviousPeriod(Sum(amount))", hourly.clone()).is_ok());
        assert!(in_chart("SamePeriodLastYear(Sum(amount))", hourly).is_err());
    }

    #[test]
    fn fixed_joins_back_an_aggregate_at_columns_the_chart_groups_by() {
        let c = in_chart("Sum(amount) / Fixed([label], Sum(amount))", by_label()).unwrap();
        assert_eq!(c.level, Level::Aggregate);
        let f = &c.staged.fixed[0];
        assert_eq!(
            (f.columns.clone(), f.agg_sql.as_str()),
            (vec!["label".to_owned()], "sum(amount)")
        );
        assert_eq!(
            c.sql,
            format!("(sum(amount) / nullIf(max({}), 0))", f.alias)
        );
        assert!(c.reads.contains(&f.alias));
        assert!(c.reads.contains("amount"));
        // A grand total needs no columns.
        let total = in_chart("Sum(amount) / Fixed(Sum(amount))", by_label()).unwrap();
        assert!(total.staged.fixed[0].columns.is_empty());
    }

    #[test]
    fn each_misuse_of_fixed_is_refused_at_the_argument() {
        let at = |f: &str, ctx: ChartCtx| in_chart(f, ctx).expect_err(f);
        let e = at("Fixed([qty], Sum(amount))", by_label());
        assert_eq!(e.position, 6, "{e:?}");
        assert!(e.message.contains("groups by label"), "{e:?}");
        let e = at("Fixed([nope], Sum(amount))", by_label());
        assert!(e.message.contains("not a column"), "{e:?}");
        let e = at("Fixed([label], amount)", by_label());
        assert_eq!(e.position, 15, "{e:?}");
        let e = at("Fixed([label], [label], Sum(amount))", by_label());
        assert!(e.message.contains("twice"), "{e:?}");
        let e = at("Fixed(label + 'x', Sum(amount))", by_label());
        assert!(e.message.contains("write a column"), "{e:?}");
        let grouped_date = ChartCtx {
            dimension: Some("day".to_owned()),
            breakdown: None,
            order_key: "day".to_owned(),
            grain: Some(Grain::Month),
        };
        let e = at("Fixed([day], Sum(amount))", grouped_date);
        assert!(e.message.contains("groups by date"), "{e:?}");
        let e = at("Fixed([label], RunningTotal(Sum(amount)))", by_label());
        assert!(e.message.contains("inside Fixed"), "{e:?}");
        // A field is not a column.
        let cols = sample_columns();
        let time = TimeContext::default();
        let scope = Scope::new(&cols, &time)
            .with_field("profit", "amount - qty")
            .with_chart(by_label());
        assert!(
            compile("Fixed([profit], Sum(amount))", &scope, Some("f"))
                .unwrap_err()
                .message
                .contains("not a column")
        );
    }

    #[test]
    fn the_new_functions_never_put_typed_text_into_sql() {
        // Hostile text in every argument position of the new functions: each
        // is refused with a position, or compiled to SQL that passes the same
        // audit as every other formula.
        let mut rng = Rng(0x2545_F491_4F6C_DD1D);
        let mut compiled = 0_u32;
        for _ in 0..4000 {
            let text = nasty(&mut rng, 30);
            let quoted = text.replace('\'', "''");
            for src in [
                format!("RunningTotal(Sum([{text}]))"),
                format!("Offset(Sum(amount), {text})"),
                format!("MovingAverage(Sum(amount), '{quoted}')"),
                format!("Fixed([{text}], Sum(amount))"),
                format!("Fixed([label], Concat(Max(label), '{quoted}'))"),
                format!("PreviousPeriod(Max(Concat(label, '{quoted}')))"),
                format!("Rank(CountDistinct(Concat('{quoted}', label)))"),
                format!("Sum(amount) / Fixed({text})"),
            ] {
                for ctx in [by_label(), by_month()] {
                    if let Ok(c) = in_chart(&src, ctx) {
                        audit_sql(&c.sql);
                        for h in &c.staged.aggs {
                            audit_sql(&h.sql);
                            assert!(
                                h.alias
                                    .chars()
                                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
                            );
                        }
                        for f in &c.staged.fixed {
                            audit_sql(&f.agg_sql);
                            assert!(f.columns.iter().all(|col| Ident::new(col.as_str()).is_ok()));
                        }
                        compiled += 1;
                    }
                }
            }
        }
        assert!(compiled > 1000, "{compiled}");
    }

    /// Not a check: writes one `SELECT` per catalog function (its example,
    /// compiled) plus the edge cases of `BI-8` section 5, for the developer
    /// to run by hand against the real engine over a scratch table with the
    /// sample columns. `FORMULA_STATEMENTS_OUT=<file> cargo test -p
    /// lakehouse-bi --lib print_engine_statements -- --ignored`.
    #[test]
    #[ignore = "writes statements for a by-hand run on the real engine"]
    fn print_engine_statements() {
        let Ok(path) = std::env::var("FORMULA_STATEMENTS_OUT") else {
            return;
        };
        let mut lines = Vec::new();
        let mut sources: Vec<String> = catalog::CATALOG
            .iter()
            .map(|f| f.example.to_owned())
            .collect();
        sources.extend(
            [
                "amount / 0",
                "amount / (qty - qty)",
                "Mod(amount, 0)",
                "Sqrt(0 - amount)",
                "Log(0)",
                "Log(0 - 1)",
                "If(IsNull(label), 'none', label)",
                "Coalesce(label, 'x')",
                "Percentile(amount, 0)",
                "Percentile(amount, 1)",
                "Percentile(amount, 0.5)",
                "DateDiff('day', ToDate('2026-01-01'), ToDate('2026-01-11'))",
                "DateDiff('day', ToDate('2026-01-11'), ToDate('2026-01-01'))",
                "DateDiff('month', ToDate('2026-01-31'), ToDate('2026-02-01'))",
                "Today()",
                "Round(2.5)",
                "Round(1234.5678, 2)",
                "Round(1234.5678, -2)",
                "Left(label, 0)",
                "Right(label, 0)",
                "Right(label, 100)",
                "Left(label, -3)",
                "Substring(label, 0)",
                "Substring(label, 2, -1)",
                "ToNumber('abc')",
                "ToDate('abc')",
                "Concat('a', Concat(label, 1))",
                "Count()",
                "Count(label)",
                "Sum(amount) / Count()",
                "Sum(amount) / 0",
                "Sum(qty - 100)",
            ]
            .map(str::to_owned),
        );
        for src in &sources {
            let aggregate = ok(src).level == Level::Aggregate;
            let sql = ok(src).sql;
            let from = "scratch_bi8.t";
            lines.push(if aggregate {
                format!("SELECT {sql} AS v FROM {from}")
            } else {
                format!("SELECT {sql} AS v FROM {from} LIMIT 3")
            });
            lines.push(format!("-- {src}"));
        }
        std::fs::write(path, lines.join("\n")).unwrap();
    }
}
