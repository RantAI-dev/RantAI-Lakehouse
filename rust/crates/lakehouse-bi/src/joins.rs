//! Joins without writing SQL (`BI-7`, with the `BI-7` part of `AI-6`): a chart
//! on a mart may read up to four more tables of the serving layer, and
//! everything downstream (dimension, breakdown, measures, filters, grain,
//! tables, pivots, steps) reads the result as one flat table.
//!
//! # Why the shapes are what they are
//!
//! - **One flat relation.** [`plan`] turns the base table and its joins into a
//!   [`Relation::Joined`] that projects the base table's columns under their own
//!   names and each joined table's columns under `<alias>__<column>`, where the
//!   alias is the table's name (a number is appended when the same table is
//!   joined twice). A base column and a joined column never collide: an alias
//!   that would produce a name already taken is numbered further.
//! - **Permissions live in the statement.** Every table is read as a plain
//!   `serving.<table> AS <alias>` reference in a `FROM` or `JOIN` position, which
//!   the role rewrite (`sql_rewrite::enforce`) replaces by the table's masked,
//!   row-filtered projection wherever the table appears. Nothing here wraps a
//!   table in a shape that hides the reference from it (no table function, no
//!   `USING`-only trick, no dictionary): a join condition therefore compares
//!   the *masked* values, never the raw ones.
//! - **Nothing is silently cut.** The statement runs under the engine's own
//!   join memory limit; an engine refusal reaches the tile as the fixed message
//!   with a reference. Outer joins run with `join_use_nulls = 1`, so a missing
//!   side is empty, not the type's default (`0`, `''`).
//! - **Calculated fields.** A field belongs to one table: a formula is compiled
//!   against the base table's columns only, so one that names a joined column is
//!   refused by the formula checker with its own positioned message.
//!
//! # Safety of the SQL
//!
//! Nothing here formats caller text into SQL. Table and column names are
//! [`Ident`]s checked against the schema the caller read from `system.columns`;
//! the join type and the comparison operators are closed sets mapped to fixed
//! keywords.

use std::collections::{HashMap, HashSet};

use lakehouse_core::ident::Ident;
use serde::{Deserialize, Serialize};

use crate::builder::{Relation, RelationColumns};
use crate::filters::ColumnKind;

/// Most tables joined to the base table.
pub const MAX_JOINS: usize = 4;

/// Most column pairs in one join's condition.
pub const MAX_PAIRS: usize = 4;

/// One pair of columns of a join condition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinPair {
    /// A column of the base table, or of an earlier join in its flat form
    /// `<alias>__<column>`.
    pub left: String,
    /// `=`, `<`, `<=`, `>` or `>=`.
    pub op: String,
    /// A column of the table being joined.
    pub right: String,
}

/// One joined table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinSpec {
    /// A table of the serving layer, with or without the `serving.` prefix.
    pub mart: String,
    /// `inner`, `left`, `right` or `full`.
    #[serde(rename = "type")]
    pub join_type: String,
    /// The condition: 1 to [`MAX_PAIRS`] pairs, AND-ed, at least one `=`.
    pub on: Vec<JoinPair>,
}

/// The four joins a chart may ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinType {
    /// Only rows with a match on both sides.
    Inner,
    /// Every row of what comes before.
    Left,
    /// Every row of the joined table.
    Right,
    /// Every row of both.
    Full,
}

impl JoinType {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "inner" => Some(Self::Inner),
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            "full" => Some(Self::Full),
            _ => None,
        }
    }

    /// The fixed keyword the statement uses.
    fn keyword(self) -> &'static str {
        match self {
            Self::Inner => "INNER JOIN",
            Self::Left => "LEFT JOIN",
            Self::Right => "RIGHT JOIN",
            Self::Full => "FULL JOIN",
        }
    }
}

/// The comparison of a pair as its fixed SQL operator.
fn operator(raw: &str) -> Option<&'static str> {
    match raw {
        "=" => Some("="),
        "<" => Some("<"),
        "<=" => Some("<="),
        ">" => Some(">"),
        ">=" => Some(">="),
        _ => None,
    }
}

/// One pair of a planned join condition: the left operand is a column of the
/// table at `left_table` (0 is the base table, `n` the `n`-th join).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedPair {
    /// Index of the table the left column belongs to.
    pub left_table: usize,
    /// The left column, under its own name in that table.
    pub left: Ident,
    /// The fixed operator.
    pub op: &'static str,
    /// The column of the joined table.
    pub right: Ident,
}

/// A joined table as the statement reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinedTable {
    /// The table, without the `serving.` prefix.
    pub mart: Ident,
    /// The prefix of its columns in the flat relation.
    pub alias: Ident,
    /// How it is joined.
    pub join_type: JoinType,
    /// The condition.
    pub on: Vec<PlannedPair>,
    /// Its columns, projected as `<alias>__<column>`.
    pub columns: Vec<Ident>,
}

/// The table alias the statement gives the `i`-th table (0 is the base).
fn table_alias(i: usize) -> String {
    format!("__jn{i}")
}

impl Relation {
    /// A base table and its joins, as [`plan`] builds them.
    pub(crate) fn render_joined(
        base: &Ident,
        base_columns: &[Ident],
        joins: &[JoinedTable],
    ) -> String {
        let mut select: Vec<String> = base_columns
            .iter()
            .map(|c| format!("{}.{c} AS {c}", table_alias(0)))
            .collect();
        let mut from = format!("serving.{base} AS {}", table_alias(0));
        for (n, j) in joins.iter().enumerate() {
            let i = n + 1;
            let a = table_alias(i);
            select.extend(
                j.columns
                    .iter()
                    .map(|c| format!("{a}.{c} AS {}__{c}", j.alias)),
            );
            let on =
                j.on.iter()
                    .map(|p| {
                        format!(
                            "{}.{} {} {a}.{}",
                            table_alias(p.left_table),
                            p.left,
                            p.op,
                            p.right
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" AND ");
            from = format!(
                "{from} {} serving.{} AS {a} ON {on}",
                j.join_type.keyword(),
                j.mart
            );
        }
        format!("(SELECT {} FROM {from}) AS jn", select.join(", "))
    }
}

/// What [`plan`] returns.
#[derive(Debug, Clone)]
pub struct Planned {
    /// The flat relation.
    pub from: Relation,
    /// The columns of the base table and of every joined table, by their flat
    /// names, with their kinds.
    pub kinds: RelationColumns,
    /// The flat names that come from the joined tables (not the base table's),
    /// in order.
    pub joined: Vec<String>,
    /// For every flat name, the table and the column it comes from.
    pub origins: HashMap<String, (String, String)>,
}

/// Whether two columns can be compared in a join condition: the same kind, or
/// both a date or a timestamp.
fn kinds_match(a: ColumnKind, b: ColumnKind) -> bool {
    a == b || (a.is_temporal() && b.is_temporal())
}

fn kind_word(kind: ColumnKind) -> &'static str {
    match kind {
        ColumnKind::Number => "a number",
        ColumnKind::Date => "a date",
        ColumnKind::DateTime => "a timestamp",
        ColumnKind::Text => "text",
    }
}

/// The columns of `mart` as sorted valid identifiers.
fn columns_of<H: std::hash::BuildHasher, S: std::hash::BuildHasher>(
    lookup: &HashMap<String, RelationColumns<H>, S>,
    mart: &str,
) -> Result<Vec<(Ident, ColumnKind)>, String> {
    let cols = lookup
        .get(mart)
        .filter(|c| !c.is_empty())
        .ok_or_else(|| format!("Gold mart '{mart}' not found in serving."))?;
    let mut sorted: Vec<(&String, &ColumnKind)> = cols.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    sorted
        .into_iter()
        .map(|(name, kind)| {
            Ident::new(name.as_str()).map(|i| (i, *kind)).map_err(|_| {
                format!(
                    "the column '{name}' of '{mart}' has a name that cannot be used in a join; rename it in the table or join another one."
                )
            })
        })
        .collect()
}

/// Everything known about the flat relation while the joins are being added.
struct Flat {
    /// Flat name to `(table index, column in that table)`.
    origin: HashMap<String, (usize, Ident)>,
    kinds: RelationColumns,
}

/// Plan the joins of a chart on `base`. `lookup` maps each table of the
/// serving layer to its columns; a table it does not know is refused.
///
/// # Errors
///
/// The sentence for the person who saved the chart: too many joins or pairs, an
/// unknown table, join type, operator or column, columns that cannot be
/// compared, a condition without `=`, or a name that is not a valid identifier.
pub fn plan<H: std::hash::BuildHasher, S: std::hash::BuildHasher>(
    base: &str,
    joins: &[JoinSpec],
    lookup: &HashMap<String, RelationColumns<H>, S>,
) -> Result<Planned, String> {
    if joins.is_empty() || joins.len() > MAX_JOINS {
        return Err(format!("a chart joins 1 to {MAX_JOINS} tables."));
    }
    let base_ident = Ident::new(base).map_err(|_| format!("invalid mart name: {base}"))?;
    let base_cols = columns_of(lookup, base)?;
    let mut flat = Flat {
        origin: HashMap::new(),
        kinds: RelationColumns::default(),
    };
    for (c, k) in &base_cols {
        flat.origin.insert(c.to_string(), (0, c.clone()));
        flat.kinds.insert(c.to_string(), *k);
    }
    let mut planned_joins: Vec<JoinedTable> = Vec::new();
    let mut joined_names: Vec<String> = Vec::new();
    let mut aliases: HashSet<String> = HashSet::new();
    for (n, spec) in joins.iter().enumerate() {
        let table = add_join(&mut flat, &mut aliases, n + 1, spec, lookup)?;
        joined_names.extend(
            table
                .columns
                .iter()
                .map(|c| format!("{}__{c}", table.alias)),
        );
        planned_joins.push(table);
    }
    let mut tables = vec![base.to_owned()];
    tables.extend(planned_joins.iter().map(|j| j.mart.as_str().to_owned()));
    let origins = flat
        .origin
        .iter()
        .map(|(name, (i, col))| (name.clone(), (tables[*i].clone(), col.to_string())))
        .collect();
    Ok(Planned {
        origins,
        from: Relation::Joined {
            base: base_ident,
            base_columns: base_cols.into_iter().map(|(c, _)| c).collect(),
            joins: planned_joins,
        },
        kinds: flat.kinds,
        joined: joined_names,
    })
}

/// The relation a chart on the mart `base` reads, joins included, and the
/// columns that go with it. `base_cols` are the base table's own columns: a
/// calculated field is compiled against those alone, so a formula cannot name a
/// joined column.
pub struct ChartRelation {
    /// The mart, or the joined tables.
    pub from: Relation,
    /// The columns of `from`, base and joined.
    pub cols: RelationColumns,
    /// The flat names that come from joined tables (empty without joins).
    pub joined: Vec<String>,
    /// For every flat name, the table and the column it comes from (empty
    /// without joins).
    pub origins: HashMap<String, (String, String)>,
}

/// [`ChartRelation`] for `base` with `joins` (none: the mart itself).
///
/// # Errors
///
/// As [`plan`].
pub fn chart_relation<H: std::hash::BuildHasher, S: std::hash::BuildHasher>(
    base: &Ident,
    joins: &[JoinSpec],
    lookup: &HashMap<String, RelationColumns<H>, S>,
) -> Result<ChartRelation, String> {
    if joins.is_empty() {
        return Ok(ChartRelation {
            from: Relation::Mart(base.clone()),
            cols: lookup
                .get(base.as_str())
                .map(|c| c.iter().map(|(k, v)| (k.clone(), *v)).collect())
                .unwrap_or_default(),
            joined: Vec::new(),
            origins: HashMap::new(),
        });
    }
    let planned = plan(base.as_str(), joins, lookup)?;
    Ok(ChartRelation {
        from: planned.from,
        cols: planned.kinds,
        joined: planned.joined,
        origins: planned.origins,
    })
}

/// `cols` with the joined columns added, refusing a calculated field that took
/// the name of one.
///
/// # Errors
///
/// The sentence for the person: a field's name is also a joined column's.
pub fn with_joined_columns<H: std::hash::BuildHasher>(
    mut cols: RelationColumns<H>,
    all: &RelationColumns,
    joined: &[String],
) -> Result<RelationColumns<H>, String> {
    for name in joined {
        if cols.contains_key(name) {
            return Err(format!(
                "the calculated field '{name}' has the name of a joined column; rename the field."
            ));
        }
        if let Some(kind) = all.get(name) {
            cols.insert(name.clone(), *kind);
        }
    }
    Ok(cols)
}

/// Check the `n`-th join (1-based) and add its columns to `flat`.
fn add_join<H: std::hash::BuildHasher, S: std::hash::BuildHasher>(
    flat: &mut Flat,
    aliases: &mut HashSet<String>,
    n: usize,
    spec: &JoinSpec,
    lookup: &HashMap<String, RelationColumns<H>, S>,
) -> Result<JoinedTable, String> {
    let mart_name = spec.mart.strip_prefix("serving.").unwrap_or(&spec.mart);
    let mart = Ident::new(mart_name).map_err(|_| format!("join {n}: invalid table name."))?;
    let join_type = JoinType::parse(&spec.join_type).ok_or_else(|| {
        format!(
            "join {n}: invalid type '{}': use inner, left, right or full.",
            spec.join_type
        )
    })?;
    if spec.on.is_empty() || spec.on.len() > MAX_PAIRS {
        return Err(format!(
            "join {n}: match on 1 to {MAX_PAIRS} pairs of columns."
        ));
    }
    if let Some(bad) = spec.on.iter().find(|p| operator(&p.op).is_none()) {
        return Err(format!(
            "join {n}: invalid comparison '{}': use =, <, <=, > or >=.",
            bad.op
        ));
    }
    if !spec.on.iter().any(|p| p.op == "=") {
        return Err(format!(
            "join {n}: at least one pair must use equals; a join on comparisons alone is refused."
        ));
    }
    let cols = columns_of(lookup, mart_name)?;
    let right_kinds: HashMap<&str, ColumnKind> =
        cols.iter().map(|(c, k)| (c.as_str(), *k)).collect();
    let on = spec
        .on
        .iter()
        .map(|p| plan_pair(flat, n, p, &right_kinds))
        .collect::<Result<Vec<_>, _>>()?;
    let alias = pick_alias(flat, aliases, mart_name, &cols)?;
    for (c, k) in &cols {
        let name = format!("{alias}__{c}");
        flat.origin.insert(name.clone(), (n, c.clone()));
        flat.kinds.insert(name, *k);
    }
    Ok(JoinedTable {
        mart,
        alias,
        join_type,
        on,
        columns: cols.into_iter().map(|(c, _)| c).collect(),
    })
}

/// One pair of join `n`: its left column is in the flat relation so far, its
/// right column is in the joined table, and the two can be compared.
fn plan_pair(
    flat: &Flat,
    n: usize,
    p: &JoinPair,
    right_kinds: &HashMap<&str, ColumnKind>,
) -> Result<PlannedPair, String> {
    let op = operator(&p.op).ok_or_else(|| {
        format!(
            "join {n}: invalid comparison '{}': use =, <, <=, > or >=.",
            p.op
        )
    })?;
    let (left_table, left) = flat.origin.get(&p.left).cloned().ok_or_else(|| {
        format!(
            "join {n}: '{}' is not a column of the base table or of an earlier join.",
            p.left
        )
    })?;
    let right_kind = right_kinds.get(p.right.as_str()).copied().ok_or_else(|| {
        format!(
            "join {n}: '{}' is not a column of the table being joined.",
            p.right
        )
    })?;
    let right = Ident::new(p.right.as_str())
        .map_err(|_| format!("join {n}: '{}' is not a usable column name.", p.right))?;
    let left_kind = flat.kinds.get(&p.left).copied().unwrap_or(ColumnKind::Text);
    if !kinds_match(left_kind, right_kind) {
        return Err(format!(
            "join {n}: '{}' is {} and '{}' is {}; they cannot be matched.",
            p.left,
            kind_word(left_kind),
            p.right,
            kind_word(right_kind)
        ));
    }
    Ok(PlannedPair {
        left_table,
        left,
        op,
        right,
    })
}

/// The alias of a joined table: its name, with a number appended when the name
/// is already an alias or one of its flat column names would collide with a
/// column already there.
fn pick_alias(
    flat: &Flat,
    aliases: &mut HashSet<String>,
    mart: &str,
    cols: &[(Ident, ColumnKind)],
) -> Result<Ident, String> {
    for attempt in 1..=(MAX_JOINS + 2) {
        let candidate = if attempt == 1 {
            mart.to_owned()
        } else {
            format!("{mart}{attempt}")
        };
        let clash = aliases.contains(&candidate)
            || cols
                .iter()
                .any(|(c, _)| flat.kinds.contains_key(&format!("{candidate}__{c}")));
        if clash {
            continue;
        }
        let alias = Ident::new(candidate.as_str())
            .map_err(|_| "a joined table's name cannot be used as a prefix.".to_owned())?;
        aliases.insert(candidate);
        return Ok(alias);
    }
    Err(format!(
        "the table '{mart}' cannot be given a name that does not clash with the columns already joined."
    ))
}

/// One column of a table, as the suggestions read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnMeta {
    /// The column's name.
    pub name: String,
    /// Its kind, from its type.
    pub kind: ColumnKind,
    /// Whether it is part of the table's sorting key.
    pub in_sorting_key: bool,
}

/// Why a pair of columns is suggested, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionRank {
    /// Same name and compatible kind, both in their table's sorting key.
    Keys,
    /// Same name and compatible kind.
    NameAndType,
    /// Same name only; the kinds cannot be matched, so a join on it is refused.
    Name,
}

/// One suggested pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Suggestion {
    /// A column of the left table.
    pub left: String,
    /// The column of the right table.
    pub right: String,
    /// Why it is suggested.
    pub rank: SuggestionRank,
    /// Whether the console should apply it by default: the pairs of the best
    /// rank that can be matched.
    pub suggested: bool,
}

/// What the suggestions route answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Suggestions {
    /// Candidate pairs, best first, then by name.
    pub pairs: Vec<Suggestion>,
    /// The right table's sorting key, in key order not guaranteed (a set).
    #[serde(rename = "rightKey")]
    pub right_key: Vec<String>,
    /// Whether the right-hand columns of the `suggested` pairs cover the right
    /// table's whole sorting key. When they do not (or the table has no key), a
    /// left row may match several right rows, so sums over the joined table's
    /// columns can multiply; the console warns, it does not block.
    #[serde(rename = "coversRightKey")]
    pub covers_right_key: bool,
}

/// Candidate join pairs between two tables, ranked: both columns in their
/// table's sorting key with the same name and a compatible kind; the same name
/// and a compatible kind; the same name. Reads only the metadata the caller
/// can already see through the `fields` route.
#[must_use]
pub fn suggest(left: &[ColumnMeta], right: &[ColumnMeta]) -> Suggestions {
    let mut pairs: Vec<Suggestion> = Vec::new();
    for l in left {
        for r in right.iter().filter(|r| r.name == l.name) {
            let compatible = kinds_match(l.kind, r.kind);
            let rank = match (compatible, l.in_sorting_key && r.in_sorting_key) {
                (true, true) => SuggestionRank::Keys,
                (true, false) => SuggestionRank::NameAndType,
                (false, _) => SuggestionRank::Name,
            };
            pairs.push(Suggestion {
                left: l.name.clone(),
                right: r.name.clone(),
                rank,
                suggested: false,
            });
        }
    }
    pairs.sort_by(|a, b| (a.rank as u8, &a.left).cmp(&(b.rank as u8, &b.left)));
    // The default: every pair of the best rank that can be matched. When the
    // best is only a name and type match, the first alone (applying every
    // shared name would over-constrain the join).
    if let Some(best) = pairs
        .first()
        .map(|p| p.rank)
        .filter(|r| *r != SuggestionRank::Name)
    {
        let most = if best == SuggestionRank::NameAndType {
            1
        } else {
            usize::MAX
        };
        for p in pairs.iter_mut().filter(|p| p.rank == best).take(most) {
            p.suggested = true;
        }
    }
    let right_key: Vec<String> = right
        .iter()
        .filter(|c| c.in_sorting_key)
        .map(|c| c.name.clone())
        .collect();
    let chosen: HashSet<&str> = pairs
        .iter()
        .filter(|p| p.suggested)
        .map(|p| p.right.as_str())
        .collect();
    let covers_right_key =
        !right_key.is_empty() && right_key.iter().all(|k| chosen.contains(k.as_str()));
    Suggestions {
        pairs,
        right_key,
        covers_right_key,
    }
}

/// Whether equality pairs on `right_columns` cover `right_key`: the check the
/// console repeats for the pairs a person edits. Exposed for the tests that
/// pin both sides to one rule.
#[must_use]
pub fn covers_key(right_key: &[String], right_columns: &[&str]) -> bool {
    !right_key.is_empty()
        && right_key
            .iter()
            .all(|k| right_columns.contains(&k.as_str()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn lookup() -> HashMap<String, RelationColumns> {
        let visits: RelationColumns = [
            ("provinsi", ColumnKind::Text),
            ("visitors", ColumnKind::Number),
            ("visit_date", ColumnKind::Date),
        ]
        .into_iter()
        .map(|(n, k)| (n.to_owned(), k))
        .collect();
        let capacity: RelationColumns = [
            ("provinsi", ColumnKind::Text),
            ("rooms", ColumnKind::Number),
            ("opened", ColumnKind::DateTime),
        ]
        .into_iter()
        .map(|(n, k)| (n.to_owned(), k))
        .collect();
        HashMap::from([
            ("visits".to_owned(), visits),
            ("capacity".to_owned(), capacity),
        ])
    }

    fn pair(left: &str, op: &str, right: &str) -> JoinPair {
        JoinPair {
            left: left.to_owned(),
            op: op.to_owned(),
            right: right.to_owned(),
        }
    }

    fn join(mart: &str, join_type: &str, on: Vec<JoinPair>) -> JoinSpec {
        JoinSpec {
            mart: mart.to_owned(),
            join_type: join_type.to_owned(),
            on,
        }
    }

    fn sql(joins: &[JoinSpec]) -> String {
        plan("visits", joins, &lookup()).unwrap().from.render()
    }

    #[test]
    fn each_join_type_renders_its_fixed_keyword_over_plain_table_references() {
        for (word, keyword) in [
            ("inner", "INNER JOIN"),
            ("left", "LEFT JOIN"),
            ("right", "RIGHT JOIN"),
            ("full", "FULL JOIN"),
        ] {
            let sql = sql(&[join(
                "capacity",
                word,
                vec![pair("provinsi", "=", "provinsi")],
            )]);
            assert_eq!(
                sql,
                format!(
                    "(SELECT __jn0.provinsi AS provinsi, __jn0.visit_date AS visit_date, __jn0.visitors AS visitors, __jn1.opened AS capacity__opened, __jn1.provinsi AS capacity__provinsi, __jn1.rooms AS capacity__rooms FROM serving.visits AS __jn0 {keyword} serving.capacity AS __jn1 ON __jn0.provinsi = __jn1.provinsi) AS jn"
                )
            );
        }
    }

    #[test]
    fn each_operator_is_a_fixed_token_and_the_prefix_of_the_table_name_is_taken() {
        for op in ["=", "<", "<=", ">", ">="] {
            let sql = sql(&[join(
                "serving.capacity",
                "left",
                vec![
                    pair("provinsi", "=", "provinsi"),
                    pair("visit_date", op, "opened"),
                ],
            )]);
            assert!(
                sql.contains(&format!(
                    "ON __jn0.provinsi = __jn1.provinsi AND __jn0.visit_date {op} __jn1.opened"
                )),
                "{op}: {sql}"
            );
        }
    }

    #[test]
    fn the_flat_relation_has_the_base_columns_and_the_prefixed_joined_ones_with_their_kinds() {
        let planned = plan(
            "visits",
            &[join(
                "capacity",
                "left",
                vec![pair("provinsi", "=", "provinsi")],
            )],
            &lookup(),
        )
        .unwrap();
        assert_eq!(planned.kinds.len(), 6);
        assert_eq!(planned.kinds["visitors"], ColumnKind::Number);
        assert_eq!(planned.kinds["capacity__rooms"], ColumnKind::Number);
        assert_eq!(planned.kinds["capacity__provinsi"], ColumnKind::Text);
        assert_eq!(
            planned.joined,
            ["capacity__opened", "capacity__provinsi", "capacity__rooms"]
        );
        assert!(planned.from.settings().contains("join_use_nulls = 1"));
    }

    #[test]
    fn two_joins_chain_and_a_later_join_can_match_a_column_of_an_earlier_one() {
        let sql = sql(&[
            join("capacity", "left", vec![pair("provinsi", "=", "provinsi")]),
            join(
                "capacity",
                "inner",
                vec![pair("capacity__provinsi", "=", "provinsi")],
            ),
        ]);
        assert!(sql.contains("__jn2.rooms AS capacity2__rooms"), "{sql}");
        assert!(
            sql.contains("INNER JOIN serving.capacity AS __jn2 ON __jn1.provinsi = __jn2.provinsi"),
            "{sql}"
        );
    }

    #[test]
    fn the_same_table_joined_twice_gets_a_numbered_prefix() {
        let planned = plan(
            "visits",
            &[
                join("capacity", "left", vec![pair("provinsi", "=", "provinsi")]),
                join("capacity", "left", vec![pair("provinsi", "=", "provinsi")]),
            ],
            &lookup(),
        )
        .unwrap();
        assert!(planned.kinds.contains_key("capacity__rooms"));
        assert!(planned.kinds.contains_key("capacity2__rooms"));
        let sql = planned.from.render();
        assert!(sql.contains("serving.capacity AS __jn1"), "{sql}");
        assert!(sql.contains("serving.capacity AS __jn2"), "{sql}");
    }

    #[test]
    fn a_prefix_that_would_collide_with_a_base_column_is_numbered_further() {
        let mut tables = lookup();
        let mut base = tables["visits"].clone();
        base.insert("capacity__rooms".to_owned(), ColumnKind::Number);
        tables.insert("visits".to_owned(), base);
        let planned = plan(
            "visits",
            &[join(
                "capacity",
                "left",
                vec![pair("provinsi", "=", "provinsi")],
            )],
            &tables,
        )
        .unwrap();
        assert!(planned.kinds.contains_key("capacity2__rooms"));
        assert_eq!(planned.kinds.len(), 3 + 1 + 3);
    }

    #[test]
    fn each_refusal_is_a_plain_sentence() {
        let l = lookup();
        let ok = || vec![pair("provinsi", "=", "provinsi")];
        let cases: Vec<(Vec<JoinSpec>, &str)> = vec![
            (vec![], "1 to 4 tables"),
            (vec![join("capacity", "left", ok()); 5], "1 to 4 tables"),
            (vec![join("nothing", "left", ok())], "not found in serving"),
            (
                vec![join("capacity", "cross", ok())],
                "invalid type 'cross'",
            ),
            (vec![join("capacity", "left", vec![])], "1 to 4 pairs"),
            (
                vec![join(
                    "capacity",
                    "left",
                    vec![pair("provinsi", "=", "provinsi"); 5],
                )],
                "1 to 4 pairs",
            ),
            (
                vec![join(
                    "capacity",
                    "left",
                    vec![pair("provinsi", "!=", "provinsi")],
                )],
                "invalid comparison '!='",
            ),
            (
                vec![join(
                    "capacity",
                    "left",
                    vec![pair("visit_date", ">", "opened")],
                )],
                "at least one pair must use equals",
            ),
            (
                vec![join(
                    "capacity",
                    "left",
                    vec![pair("nope", "=", "provinsi")],
                )],
                "'nope' is not a column of the base table",
            ),
            (
                vec![join(
                    "capacity",
                    "left",
                    vec![pair("provinsi", "=", "nope")],
                )],
                "'nope' is not a column of the table being joined",
            ),
            (
                vec![join(
                    "capacity",
                    "left",
                    vec![pair("visitors", "=", "provinsi")],
                )],
                "'visitors' is a number and 'provinsi' is text",
            ),
            (
                vec![join(
                    "capacity",
                    "left",
                    vec![pair("capacity__rooms", "=", "rooms")],
                )],
                "not a column of the base table or of an earlier join",
            ),
        ];
        for (joins, expected) in cases {
            let err = plan("visits", &joins, &l).unwrap_err();
            assert!(err.contains(expected), "{err} should contain {expected}");
        }
        assert!(
            plan("missing", &[join("capacity", "left", ok())], &l)
                .unwrap_err()
                .contains("not found in serving")
        );
    }

    #[test]
    fn a_date_matches_a_timestamp_and_a_column_with_an_unusable_name_is_refused() {
        let ok = plan(
            "visits",
            &[join(
                "capacity",
                "left",
                vec![
                    pair("provinsi", "=", "provinsi"),
                    pair("visit_date", ">=", "opened"),
                ],
            )],
            &lookup(),
        );
        assert!(ok.is_ok());
        let mut tables = lookup();
        tables
            .get_mut("capacity")
            .unwrap()
            .insert("bad name".to_owned(), ColumnKind::Text);
        let err = plan(
            "visits",
            &[join(
                "capacity",
                "left",
                vec![pair("provinsi", "=", "provinsi")],
            )],
            &tables,
        )
        .unwrap_err();
        assert!(err.contains("'bad name' of 'capacity'"), "{err}");
    }

    #[test]
    fn a_joins_json_is_read_with_type_and_refuses_unknown_keys() {
        let j: JoinSpec = serde_json::from_str(
            r#"{"mart":"capacity","type":"left","on":[{"left":"provinsi","op":"=","right":"provinsi"}]}"#,
        )
        .unwrap();
        assert_eq!(j.join_type, "left");
        assert!(
            serde_json::from_str::<JoinSpec>(r#"{"mart":"c","type":"left","on":[],"using":"x"}"#)
                .is_err()
        );
        assert_eq!(serde_json::to_value(&j).unwrap()["type"], "left");
    }

    fn meta(name: &str, kind: ColumnKind, key: bool) -> ColumnMeta {
        ColumnMeta {
            name: name.to_owned(),
            kind,
            in_sorting_key: key,
        }
    }

    #[test]
    fn suggestions_rank_keys_first_then_name_and_type_then_name_and_flag_a_missing_key() {
        let left = [
            meta("provinsi", ColumnKind::Text, true),
            meta("visit_date", ColumnKind::Date, false),
            meta("rooms", ColumnKind::Text, false),
        ];
        let right = [
            meta("provinsi", ColumnKind::Text, true),
            meta("visit_date", ColumnKind::DateTime, false),
            meta("rooms", ColumnKind::Number, false),
        ];
        let s = suggest(&left, &right);
        let order: Vec<(&str, SuggestionRank, bool)> = s
            .pairs
            .iter()
            .map(|p| (p.left.as_str(), p.rank, p.suggested))
            .collect();
        assert_eq!(
            order,
            [
                ("provinsi", SuggestionRank::Keys, true),
                ("visit_date", SuggestionRank::NameAndType, false),
                ("rooms", SuggestionRank::Name, false),
            ]
        );
        assert_eq!(s.right_key, ["provinsi"]);
        assert!(s.covers_right_key);
        // No shared sorting key: the best is a name and type match, taken
        // alone, and the right key is not covered.
        let right = [
            meta("id", ColumnKind::Number, true),
            meta("visit_date", ColumnKind::Date, false),
        ];
        let s = suggest(&left, &right);
        assert_eq!(s.pairs.len(), 1);
        assert!(s.pairs[0].suggested);
        assert!(!s.covers_right_key);
        assert!(covers_key(&["id".to_owned()], &["id", "x"]));
        assert!(!covers_key(&[], &["id"]));
        // Nothing in common: no pair is suggested, and nothing is invented.
        let s = suggest(&left, &[meta("zzz", ColumnKind::Text, true)]);
        assert!(s.pairs.is_empty(), "{:?}", s.pairs);
        assert!(!s.covers_right_key);
    }
}
