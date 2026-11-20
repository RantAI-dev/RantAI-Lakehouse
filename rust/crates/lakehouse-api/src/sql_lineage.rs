//! Which source column feeds each output column of one `SELECT`
//! (`DATA-13`, finding F5).
//!
//! [`map_select`] reads a statement that fills a table (`SELECT`,
//! `INSERT INTO t [(cols)] SELECT`, `CREATE TABLE t ... AS SELECT`) and says,
//! for every output column, which column of which table it comes from and how
//! (`copy`, `cast` or `expression`). It is pure: no network, no clock.
//!
//! # Never a guess
//!
//! Column lineage is read as authoritative, so a mapping is returned only
//! when the statement itself proves it. What it cannot prove comes back as a
//! [`PartialOutput`] with a fixed sentence saying why, and the caller stores
//! that as a partial row rather than dropping it:
//!
//! * `*` or `t.*` over a table whose columns are not in `known_columns`;
//! * a column read through an unreadable `WITH` name (a recursive `WITH`, a
//!   name with its own column list, a `MATERIALIZED`/`FROM` hint, a name read
//!   before it is defined, a name defined twice, a name whose query cannot be
//!   read, or one that differs only in letter case), a subquery in `FROM` or
//!   a table function;
//! * a `UNION` (every output of the left branch is partial);
//! * an unqualified identifier that matches no table, or several, in `FROM`,
//!   or that could belong to a source whose columns are not known;
//! * an expression holding a subquery, or an unaliased expression (the engine
//!   names it, so its column name is not known here).
//!
//! # Through a `WITH`
//!
//! A `WITH` name resolves to the outputs of its query, read with the same
//! rules as a top-level `SELECT` (`DATA-57` D1). A nested `WITH` is read the
//! same way and shadows an outer name of the same spelling (D2); names match
//! exactly, case included (D3). A `*` over a name whose columns are all known
//! expands to those columns (D4). A name we cannot read stays a partial row
//! with a fixed reason, never a guessed edge (D7).
//!
//! A statement that cannot be read at all (not one statement, a kind other
//! than the three above, or a `INSERT` whose target columns are not known)
//! is an [`Unmappable`] error carrying the reason.
//!
//! Only the projection is traced. `WHERE`, `JOIN ... ON`, `GROUP BY` and
//! `ORDER BY` decide which rows an output holds, not which column feeds it,
//! so they are not lineage here. An output with no column in it (a literal,
//! `now()`) has no source and is listed in [`Lineage::constants`] only.
//!
//! Table keys are `schema.table`, lowercased, with a leading catalog part
//! dropped (`sql_rewrite::canonicalize`), the way the table graph names
//! them. Column names keep the case the statement or `known_columns` gives.

use std::collections::HashMap;
use std::convert::Infallible;
use std::fmt;
use std::ops::ControlFlow;

use sqlparser::ast::{
    CastKind, Expr, Ident, ObjectName, ObjectNamePart, Query, SelectItem,
    SelectItemQualifiedWildcardKind, SetExpr, Statement, TableFactor, TableObject, TableWithJoins,
    Visit, Visitor, WildcardAdditionalOptions,
};
use sqlparser::dialect::Dialect;
use sqlparser::parser::Parser;

use crate::sql_rewrite::canonicalize;

/// How an output is computed from its source column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Transform {
    /// A bare column, possibly renamed.
    Copy,
    /// A `CAST` of one column.
    Cast,
    /// Anything else that reads a column.
    Expression,
}

impl Transform {
    /// The word stored in `column_lineage.transform`.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Cast => "cast",
            Self::Expression => "expression",
        }
    }
}

/// One proven mapping: `output` is built from `from_table.from_column`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Mapping {
    pub(crate) output: String,
    pub(crate) from_table: String,
    pub(crate) from_column: String,
    pub(crate) transform: Transform,
}

/// An output, or part of one, the statement does not let us trace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PartialOutput {
    pub(crate) output: String,
    pub(crate) reason: String,
    pub(crate) transform: Transform,
}

/// What [`map_select`] found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Lineage {
    /// The table an `INSERT` or `CREATE TABLE ... AS` writes, as a key.
    pub(crate) target: Option<String>,
    /// Proven mappings, in output order.
    pub(crate) mappings: Vec<Mapping>,
    /// What could not be traced, in output order.
    pub(crate) partial: Vec<PartialOutput>,
    /// Outputs that read no column at all.
    pub(crate) constants: Vec<String>,
}

/// The statement cannot be read for column lineage; the sentence says why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Unmappable(pub(crate) String);

impl fmt::Display for Unmappable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn unmappable(reason: impl Into<String>) -> Unmappable {
    Unmappable(reason.into())
}

/// An identifier shortened for a reason sentence, so a hostile or huge name
/// cannot make a row longer than its 300-character limit.
fn short(name: &str) -> String {
    let mut out: String = name.chars().take(64).collect();
    if name.chars().count() > 64 {
        out.push_str("...");
    }
    out
}

/// What one `FROM` entry is.
enum Source {
    /// A real table, by key.
    Table(String),
    /// A `WITH` name and the outputs of its query (`DATA-57` D1). The outputs
    /// are cloned: at most 25 CTEs per statement and 20,000 characters, so
    /// this is bounded, and it lets a nested query read a shadowing name.
    Cte { name: String, columns: Vec<Out> },
    /// Something whose columns are not traced; the sentence says what.
    Opaque(String),
}

struct Entry {
    /// Lowercased names a column can be qualified with: the alias, the key,
    /// and the bare table name.
    names: Vec<String>,
    source: Source,
}

/// Everything one `SELECT` needs to resolve its identifiers.
struct Scope<'a> {
    entries: Vec<Entry>,
    known: &'a HashMap<String, Vec<String>>,
    /// Aliases of other projection items, with the index that defines each
    /// (`DATA-57` D6). An unqualified name that is one of them is read by
    /// `ClickHouse` as the alias, not as a column of `FROM`.
    aliases: Vec<(String, usize)>,
    current_item: usize,
}

/// One output slot, before it is split into mappings and partials.
#[derive(Clone)]
struct Out {
    name: String,
    sources: Vec<(String, String)>,
    reasons: Vec<String>,
    transform: Transform,
    /// A `*` that could not be expanded: one slot standing for an unknown
    /// number of columns, so a positional `INSERT` cannot be read.
    unexpanded_star: bool,
}

/// What a `WITH` name stands for. An unreadable name is still in scope, as a
/// partial row with its fixed reason, so a read through it is never guessed.
#[derive(Clone)]
enum CteDef {
    Columns(Vec<Out>),
    Opaque(String),
}

/// The `WITH` names visible in one query. `defined` is walked from the end
/// (a nested name shadows an outer one of the same spelling, `DATA-57` D2);
/// `pending` holds exact names of the current `WITH` not yet read, so a
/// forward reference stays partial (D7). Both store the name as written,
/// because `ClickHouse` matches `WITH` names with exact case (D3).
struct CteEnv {
    defined: Vec<(String, CteDef)>,
    pending: Vec<String>,
}

impl CteEnv {
    fn empty() -> Self {
        Self {
            defined: Vec::new(),
            pending: Vec::new(),
        }
    }
}

/// One resolved read: the sources and reasons a column carries, and how it
/// was computed. A real table gives one source and `Copy`; a `WITH` column
/// gives its own sources, reasons and transform (`DATA-57` D5).
struct Read {
    sources: Vec<(String, String)>,
    reasons: Vec<String>,
    transform: Transform,
}

impl Read {
    fn table(pair: (String, String)) -> Self {
        Self {
            sources: vec![pair],
            reasons: Vec::new(),
            transform: Transform::Copy,
        }
    }
}

/// Compose an outer transform with the transform of a column read through a
/// `WITH` (`DATA-57` D5). `expression` absorbs anything; a `cast` of a `cast`
/// is `expression` because the stored word means one `CAST` of one column
/// (planner default).
fn compose_transforms(outer: Transform, inner: Transform) -> Transform {
    use Transform::{Cast, Copy, Expression};
    match (outer, inner) {
        (Copy, inner) => inner,
        (Cast, Copy) => Cast,
        _ => Expression,
    }
}

const MSG_SUBQUERY_EXPR: &str = "the expression contains a subquery, which is not traced";
const MSG_UNNAMED: &str =
    "the expression has no alias, so the engine chooses the column name; add an alias";
const MSG_UNION: &str = "built with UNION, INTERSECT or EXCEPT; the branches are not traced";
const MSG_SUBQUERY_FROM: &str = "read through a subquery in FROM, which is not traced";
const MSG_TABLE_FUNCTION: &str = "read through a table function or another relation that is not a plain table, which is not traced";
const MSG_WILDCARD_OPTIONS: &str =
    "the * has EXCEPT, REPLACE, RENAME or similar options, which are not traced";
const MSG_CTE_RECURSIVE: &str = "read through a recursive WITH, which is not traced";
const MSG_CTE_COLUMNS: &str =
    "read through a WITH name with its own column list, which is not traced";
const MSG_CTE_HINT: &str =
    "read through a WITH name with a MATERIALIZED or FROM hint, which is not traced";
const MSG_CTE_FORWARD: &str = "read through a WITH name before it is defined, which is not traced";
const MSG_CTE_DUPLICATE: &str =
    "read through a WITH name that is defined twice, which is not traced";
const MSG_CTE_UNREADABLE: &str =
    "read through a WITH name whose query cannot be read, which is not traced";
const MSG_CTE_CASE: &str =
    "the name differs from a WITH name only in letter case, which is not traced";
const MSG_ALIAS_CAPTURE: &str =
    "the name is also an alias in this SELECT, and ClickHouse reads the alias; rename one";

/// What one statement writes (`INSERT` and `CREATE TABLE ... AS`), an
/// `INSERT`'s explicit column list, and the query that fills it.
type Parsed = (Option<String>, Option<Vec<String>>, Query);

/// The table `sql` writes, as a key, when it is an `INSERT ... SELECT` or a
/// `CREATE TABLE ... AS SELECT`; `None` for anything else, including a
/// statement [`map_select`] would refuse. It needs no column knowledge, so a
/// caller can find the target before it fetches any columns.
pub(crate) fn statement_target(sql: &str, dialect: &dyn Dialect) -> Option<String> {
    parse_statement(sql, dialect)
        .ok()
        .and_then(|(target, ..)| target)
}

fn parse_statement(sql: &str, dialect: &dyn Dialect) -> Result<Parsed, Unmappable> {
    let mut statements =
        Parser::parse_sql(dialect, sql).map_err(|_| unmappable("the statement does not parse"))?;
    if statements.len() != 1 {
        return Err(unmappable("expected exactly one statement"));
    }
    let Some(statement) = statements.pop() else {
        return Err(unmappable("expected exactly one statement"));
    };
    // `insert_columns` is `Some` only for an INSERT: its explicit column
    // list, empty when the INSERT takes the target's columns in order.
    let (target, insert_columns, query) = match statement {
        Statement::Query(query) => (None, None, *query),
        Statement::Insert(insert) => {
            let TableObject::TableName(name) = &insert.table else {
                return Err(unmappable("the INSERT target is a table function"));
            };
            let columns: Vec<String> = insert.columns.iter().filter_map(last_part).collect();
            let Some(source) = insert.source else {
                return Err(unmappable("the INSERT has no SELECT"));
            };
            (Some(table_key(name)), Some(columns), *source)
        }
        Statement::CreateTable(create) => {
            let Some(query) = create.query else {
                return Err(unmappable("the CREATE TABLE has no SELECT"));
            };
            (Some(table_key(&create.name)), None, *query)
        }
        _ => {
            return Err(unmappable(
                "only SELECT, INSERT ... SELECT and CREATE TABLE ... AS SELECT are read",
            ));
        }
    };
    Ok((target, insert_columns, query))
}

/// Map the output columns of `sql` to their source columns.
///
/// `known_columns` maps a table key (`silver.x`) to its column names, as read
/// from the engine; it expands `*`, picks the table an unqualified column
/// belongs to among several, and names the target columns of a positional
/// `INSERT`.
///
/// # Errors
///
/// [`Unmappable`] when `sql` does not parse, is not exactly one statement of
/// a kind listed in the module doc, reads `VALUES` or another non-`SELECT`
/// body, or is an `INSERT` whose target columns are not known or do not match
/// the `SELECT` in number.
pub(crate) fn map_select(
    sql: &str,
    dialect: &dyn Dialect,
    known_columns: &HashMap<String, Vec<String>>,
) -> Result<Lineage, Unmappable> {
    let (target, insert_columns, query) = parse_statement(sql, dialect)?;
    let mut outs = outputs_of_query(&query, &CteEnv::empty(), known_columns)?;

    // An INSERT names its columns, or takes the target's in order; the
    // SELECT's own aliases do not matter to where a value lands.
    if let Some(explicit) = insert_columns {
        let names = positional_names(target.as_deref(), explicit, known_columns)?;
        if outs.iter().any(|o| o.unexpanded_star) || outs.len() != names.len() {
            return Err(unmappable(
                "the SELECT's columns cannot be matched one to one with the INSERT's columns",
            ));
        }
        for (out, name) in outs.iter_mut().zip(names) {
            out.name = name;
        }
    }
    Ok(assemble(target, outs))
}

fn positional_names(
    target: Option<&str>,
    explicit: Vec<String>,
    known: &HashMap<String, Vec<String>>,
) -> Result<Vec<String>, Unmappable> {
    if !explicit.is_empty() {
        return Ok(explicit);
    }
    let Some(target) = target else {
        return Err(unmappable("the INSERT target is not known"));
    };
    match known.get(target) {
        Some(columns) if !columns.is_empty() => Ok(columns.clone()),
        _ => Err(unmappable(format!(
            "the columns of {} are not known, so the INSERT cannot be matched by position",
            short(target)
        ))),
    }
}

fn assemble(target: Option<String>, outs: Vec<Out>) -> Lineage {
    let mut lineage = Lineage {
        target,
        ..Lineage::default()
    };
    for out in outs {
        if out.sources.is_empty() && out.reasons.is_empty() {
            lineage.constants.push(out.name);
            continue;
        }
        for (from_table, from_column) in out.sources {
            lineage.mappings.push(Mapping {
                output: out.name.clone(),
                from_table,
                from_column,
                transform: out.transform,
            });
        }
        for reason in out.reasons {
            lineage.partial.push(PartialOutput {
                output: out.name.clone(),
                reason,
                transform: out.transform,
            });
        }
    }
    lineage
}

/// The last part of a name, as written.
fn last_part(name: &ObjectName) -> Option<String> {
    name.0
        .last()
        .and_then(ObjectNamePart::as_ident)
        .map(|ident| ident.value.clone())
}

/// A table name as a key (`schema.table`), lowercased.
fn table_key(name: &ObjectName) -> String {
    let parts: Vec<String> = name
        .0
        .iter()
        .filter_map(ObjectNamePart::as_ident)
        .map(|ident| ident.value.to_ascii_lowercase())
        .collect();
    canonicalize(&parts)
}

/// Aliases of the projection items of one `SELECT`, with their index. `id AS
/// id` names the column itself and is not a capture (`DATA-57` D6); every
/// other alias hides that spelling for an unqualified read in another item.
fn projection_aliases(projection: &[SelectItem]) -> Vec<(String, usize)> {
    projection
        .iter()
        .enumerate()
        .filter_map(|(index, item)| match item {
            SelectItem::ExprWithAlias { expr, alias } => {
                if matches!(unwrap_nested(expr), Expr::Identifier(id) if id.value == alias.value) {
                    None
                } else {
                    Some((alias.value.clone(), index))
                }
            }
            _ => None,
        })
        .collect()
}

/// The outputs of a query: read its own `WITH` in order, then its body. The
/// CTEs of a `WITH` are visible to the ones after them and to the body, and a
/// nested `WITH` shadows an outer name of the same spelling (`DATA-57` D1, D2).
fn outputs_of_query(
    query: &Query,
    env: &CteEnv,
    known: &HashMap<String, Vec<String>>,
) -> Result<Vec<Out>, Unmappable> {
    let Some(with) = &query.with else {
        return outputs_of_body(&query.body, env, known);
    };

    let mut defined = env.defined.clone();
    let mut pending: Vec<String> = with
        .cte_tables
        .iter()
        .map(|cte| cte.alias.name.value.clone())
        .collect();
    // A name declared twice is opaque on both definitions (D7).
    let duplicated: Vec<String> = pending
        .iter()
        .filter(|name| pending.iter().filter(|n| *n == *name).count() > 1)
        .cloned()
        .collect();

    for cte in &with.cte_tables {
        let name = cte.alias.name.value.clone();
        let def = if with.recursive {
            CteDef::Opaque(MSG_CTE_RECURSIVE.to_owned())
        } else if duplicated.contains(&name) {
            CteDef::Opaque(MSG_CTE_DUPLICATE.to_owned())
        } else if !cte.alias.columns.is_empty() {
            CteDef::Opaque(MSG_CTE_COLUMNS.to_owned())
        } else if cte.materialized.is_some() || cte.from.is_some() {
            CteDef::Opaque(MSG_CTE_HINT.to_owned())
        } else {
            let child = CteEnv {
                defined: defined.clone(),
                pending: pending.clone(),
            };
            match outputs_of_query(&cte.query, &child, known) {
                Err(_) => CteDef::Opaque(MSG_CTE_UNREADABLE.to_owned()),
                Ok(outs) => match outs.iter().find(|o| o.unexpanded_star) {
                    // An unreadable `*` inside the CTE makes the whole name
                    // opaque with that `*`'s own reason (D4).
                    Some(star) => CteDef::Opaque(
                        star.reasons
                            .first()
                            .cloned()
                            .unwrap_or_else(|| MSG_CTE_UNREADABLE.to_owned()),
                    ),
                    None => CteDef::Columns(outs),
                },
            }
        };
        defined.push((name.clone(), def));
        pending.retain(|n| n != &name);
    }

    outputs_of_body(
        &query.body,
        &CteEnv {
            defined,
            pending: Vec::new(),
        },
        known,
    )
}

/// The outputs of a query body: a `SELECT`, a parenthesized query, or a set
/// operation (whose outputs are all partial).
fn outputs_of_body(
    body: &SetExpr,
    env: &CteEnv,
    known: &HashMap<String, Vec<String>>,
) -> Result<Vec<Out>, Unmappable> {
    match body {
        SetExpr::Select(select) => {
            let mut scope = Scope {
                entries: select_entries(&select.from, env),
                known,
                aliases: projection_aliases(&select.projection),
                current_item: 0,
            };
            let mut outs = Vec::new();
            for (index, item) in select.projection.iter().enumerate() {
                scope.current_item = index;
                outs.extend(outputs_of_item(item, &scope));
            }
            Ok(outs)
        }
        SetExpr::Query(inner) => outputs_of_query(inner, env, known),
        SetExpr::SetOperation { left, .. } => {
            let mut outs = outputs_of_body(left, env, known)?;
            if outs.iter().any(|o| o.unexpanded_star) {
                return Err(unmappable(
                    "a UNION with * cannot be read: the columns of its branches are not known",
                ));
            }
            for out in &mut outs {
                out.sources.clear();
                out.reasons = vec![MSG_UNION.to_owned()];
            }
            Ok(outs)
        }
        _ => Err(unmappable("only a SELECT body is read")),
    }
}

/// Whether a one-part name names a `WITH` of the current scope.
enum CteLookup<'a> {
    Defined(&'a CteDef),
    Forward,
    CaseOnly,
    None,
}

fn cte_lookup<'a>(env: &'a CteEnv, name: &str) -> CteLookup<'a> {
    // Walk back so a nested name shadows an outer one (D2).
    for (defined_name, def) in env.defined.iter().rev() {
        if defined_name == name {
            return CteLookup::Defined(def);
        }
    }
    if env.pending.iter().any(|n| n == name) {
        return CteLookup::Forward;
    }
    let case_only = env
        .defined
        .iter()
        .any(|(n, _)| n.eq_ignore_ascii_case(name))
        || env.pending.iter().any(|n| n.eq_ignore_ascii_case(name));
    if case_only {
        return CteLookup::CaseOnly;
    }
    CteLookup::None
}

/// Every relation of a `FROM` clause with the names it can be qualified by.
fn select_entries(from: &[TableWithJoins], env: &CteEnv) -> Vec<Entry> {
    let mut entries = Vec::new();
    for twj in from {
        push_factor(&twj.relation, env, &mut entries);
        for join in &twj.joins {
            push_factor(&join.relation, env, &mut entries);
        }
    }
    entries
}

fn push_factor(factor: &TableFactor, env: &CteEnv, entries: &mut Vec<Entry>) {
    match factor {
        TableFactor::Table {
            name, alias, args, ..
        } => {
            let key = table_key(name);
            let bare = key.rsplit('.').next().unwrap_or_default().to_owned();
            let alias_name = alias.as_ref().map(|a| a.name.value.to_ascii_lowercase());
            let mut names: Vec<String> = alias_name.into_iter().collect();
            if args.is_some() {
                entries.push(Entry {
                    names,
                    source: Source::Opaque(MSG_TABLE_FUNCTION.to_owned()),
                });
                return;
            }
            if name.0.len() == 1
                && let Some(raw) = name.0.last().and_then(ObjectNamePart::as_ident)
            {
                // CTE names match exactly, case included (D3): an
                // exact match resolves, a case-only match is opaque.
                match cte_lookup(env, &raw.value) {
                    CteLookup::Defined(CteDef::Columns(columns)) => {
                        names.push(raw.value.to_ascii_lowercase());
                        entries.push(Entry {
                            names,
                            source: Source::Cte {
                                name: raw.value.clone(),
                                columns: columns.clone(),
                            },
                        });
                        return;
                    }
                    CteLookup::Defined(CteDef::Opaque(reason)) => {
                        entries.push(Entry {
                            names,
                            source: Source::Opaque(reason.clone()),
                        });
                        return;
                    }
                    CteLookup::Forward => {
                        entries.push(Entry {
                            names,
                            source: Source::Opaque(MSG_CTE_FORWARD.to_owned()),
                        });
                        return;
                    }
                    CteLookup::CaseOnly => {
                        entries.push(Entry {
                            names,
                            source: Source::Opaque(MSG_CTE_CASE.to_owned()),
                        });
                        return;
                    }
                    CteLookup::None => {}
                }
            }
            // A table with an alias is no longer reachable by its own name in
            // standard SQL, but ClickHouse still accepts it; accepting it
            // here only widens what can be qualified, never what is guessed.
            names.push(key.clone());
            names.push(bare);
            entries.push(Entry {
                names,
                source: Source::Table(key),
            });
        }
        TableFactor::Derived { alias, .. } => entries.push(Entry {
            names: alias
                .iter()
                .map(|a| a.name.value.to_ascii_lowercase())
                .collect(),
            source: Source::Opaque(MSG_SUBQUERY_FROM.to_owned()),
        }),
        TableFactor::NestedJoin {
            table_with_joins, ..
        } => {
            push_factor(&table_with_joins.relation, env, entries);
            for join in &table_with_joins.joins {
                push_factor(&join.relation, env, entries);
            }
        }
        _ => entries.push(Entry {
            names: Vec::new(),
            source: Source::Opaque(MSG_TABLE_FUNCTION.to_owned()),
        }),
    }
}

fn outputs_of_item(item: &SelectItem, scope: &Scope<'_>) -> Vec<Out> {
    match item {
        SelectItem::UnnamedExpr(expr) => {
            let name = match expr {
                Expr::Identifier(ident) => Some(ident.value.clone()),
                Expr::CompoundIdentifier(parts) => parts.last().map(|p| p.value.clone()),
                _ => None,
            };
            if let Some(name) = name {
                return vec![analyse(&name, expr, scope)];
            }
            let mut out = analyse(&short(&expr.to_string()), expr, scope);
            // A constant stays a constant; anything that reads a column has
            // no name we can state, so only the reason is kept.
            if !(out.sources.is_empty() && out.reasons.is_empty()) {
                out.sources.clear();
                out.reasons = vec![MSG_UNNAMED.to_owned()];
            }
            vec![out]
        }
        SelectItem::ExprWithAlias { expr, alias } => vec![analyse(&alias.value, expr, scope)],
        SelectItem::ExprWithAliases { expr, .. } => {
            let mut out = analyse(&short(&expr.to_string()), expr, scope);
            out.reasons
                .push("an expression with several aliases is not traced".to_owned());
            out.sources.clear();
            vec![out]
        }
        SelectItem::Wildcard(options) => {
            let all: Vec<usize> = (0..scope.entries.len()).collect();
            expand_star(&all, options, scope)
        }
        SelectItem::QualifiedWildcard(
            SelectItemQualifiedWildcardKind::ObjectName(name),
            options,
        ) => {
            let qualifier = table_key(name);
            let raw = name
                .0
                .iter()
                .filter_map(ObjectNamePart::as_ident)
                .map(|i| i.value.to_ascii_lowercase())
                .collect::<Vec<_>>()
                .join(".");
            let hits: Vec<usize> = scope
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| e.names.contains(&raw) || e.names.contains(&qualifier))
                .map(|(i, _)| i)
                .collect();
            if hits.len() == 1 {
                expand_star(&hits, options, scope)
            } else {
                vec![unexpanded(
                    format!("{}.*", short(&raw)),
                    format!(
                        "the qualifier {} of * matches {} in FROM",
                        short(&raw),
                        if hits.is_empty() {
                            "no relation"
                        } else {
                            "several relations"
                        }
                    ),
                )]
            }
        }
        SelectItem::QualifiedWildcard(SelectItemQualifiedWildcardKind::Expr(expr), _) => {
            vec![unexpanded(
                format!("{}.*", short(&expr.to_string())),
                "a * over an expression is not traced".to_owned(),
            )]
        }
    }
}

fn unexpanded(name: String, reason: String) -> Out {
    Out {
        name,
        sources: Vec::new(),
        reasons: vec![reason],
        transform: Transform::Copy,
        unexpanded_star: true,
    }
}

/// Expand `*` over the chosen `FROM` entries: one copy per known column, or
/// one partial slot per entry whose columns are not known.
fn expand_star(
    entries: &[usize],
    options: &WildcardAdditionalOptions,
    scope: &Scope<'_>,
) -> Vec<Out> {
    if options.opt_ilike.is_some()
        || options.opt_exclude.is_some()
        || options.opt_except.is_some()
        || options.opt_replace.is_some()
        || options.opt_rename.is_some()
    {
        return vec![unexpanded("*".to_owned(), MSG_WILDCARD_OPTIONS.to_owned())];
    }
    let mut outs: Vec<Out> = Vec::new();
    for &index in entries {
        let entry = &scope.entries[index];
        match &entry.source {
            Source::Opaque(reason) => outs.push(unexpanded("*".to_owned(), reason.clone())),
            Source::Cte { columns, .. } => {
                // A CTE's columns are all known here (D4): an unexpanded `*`
                // would have made the name opaque when it was defined.
                outs.extend(columns.iter().cloned());
            }
            Source::Table(key) => match scope.known.get(key) {
                Some(columns) if !columns.is_empty() => {
                    for column in columns {
                        outs.push(Out {
                            name: column.clone(),
                            sources: vec![(key.clone(), column.clone())],
                            reasons: Vec::new(),
                            transform: Transform::Copy,
                            unexpanded_star: false,
                        });
                    }
                }
                _ => outs.push(unexpanded(
                    "*".to_owned(),
                    format!("the columns of {} are not known", short(key)),
                )),
            },
        }
    }
    // Two relations with a column of the same name: the engine renames one,
    // and which one is not decided here. A single CTE can also hold two
    // outputs of one name (D7), so its `*` must run this rule too.
    if entries.len() > 1
        || entries
            .iter()
            .any(|&i| matches!(scope.entries[i].source, Source::Cte { .. }))
    {
        let names: Vec<String> = outs.iter().map(|o| o.name.clone()).collect();
        for out in &mut outs {
            if !out.unexpanded_star && names.iter().filter(|n| **n == out.name).count() > 1 {
                out.sources.clear();
                out.reasons = vec![format!(
                    "the column {} is in more than one joined table, and the engine renames one",
                    short(&out.name)
                )];
            }
        }
    }
    outs
}

/// Collects the identifiers an expression reads and whether it holds a
/// subquery, through `sqlparser`'s derived traversal so no `Expr` variant is
/// hand-listed (and so none can be missed).
#[derive(Default)]
struct ExprReads {
    identifiers: Vec<Vec<Ident>>,
    has_subquery: bool,
}

impl Visitor for ExprReads {
    type Break = Infallible;

    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<Infallible> {
        match expr {
            Expr::Identifier(ident) => self.identifiers.push(vec![ident.clone()]),
            Expr::CompoundIdentifier(parts) => self.identifiers.push(parts.clone()),
            _ => {}
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_query(&mut self, _query: &Query) -> ControlFlow<Infallible> {
        self.has_subquery = true;
        ControlFlow::Continue(())
    }
}

fn unwrap_nested(mut expr: &Expr) -> &Expr {
    while let Expr::Nested(inner) = expr {
        expr = inner;
    }
    expr
}

fn is_column(expr: &Expr) -> bool {
    matches!(
        unwrap_nested(expr),
        Expr::Identifier(_) | Expr::CompoundIdentifier(_)
    )
}

fn transform_of(expr: &Expr) -> Transform {
    match unwrap_nested(expr) {
        e if is_column(e) => Transform::Copy,
        Expr::Cast {
            kind: CastKind::Cast | CastKind::TryCast | CastKind::DoubleColon,
            expr: inner,
            ..
        } if is_column(inner) => Transform::Cast,
        _ => Transform::Expression,
    }
}

/// Resolve every column an expression reads against the `FROM` entries.
fn analyse(name: &str, expr: &Expr, scope: &Scope<'_>) -> Out {
    let mut out = Out {
        name: name.to_owned(),
        sources: Vec::new(),
        reasons: Vec::new(),
        transform: transform_of(expr),
        unexpanded_star: false,
    };
    let mut reads = ExprReads::default();
    let _: ControlFlow<Infallible> = expr.visit(&mut reads);
    if reads.has_subquery {
        // Identifiers inside belong to the inner query's scope, so none of
        // them can be resolved against this FROM.
        out.reasons.push(MSG_SUBQUERY_EXPR.to_owned());
        return out;
    }
    let outer = out.transform;
    let mut composed = None;
    for parts in &reads.identifiers {
        match resolve(parts, scope) {
            Ok(read) => {
                for source in read.sources {
                    if !out.sources.contains(&source) {
                        out.sources.push(source);
                    }
                }
                for reason in read.reasons {
                    if !out.reasons.contains(&reason) {
                        out.reasons.push(reason);
                    }
                }
                composed = Some(compose_transforms(outer, read.transform));
            }
            Err(reason) => {
                if !out.reasons.contains(&reason) {
                    out.reasons.push(reason);
                }
            }
        }
    }
    // Only a bare column or a cast of one has a single read to compose with
    // (`transform_of` is `Copy` or `Cast`); an expression stays an expression.
    if matches!(outer, Transform::Copy | Transform::Cast)
        && let Some(transform) = composed
    {
        out.transform = transform;
    }
    out
}

/// Whether a table's known columns hold a name.
enum Lookup {
    /// Found; the name as the table spells it.
    Found(String),
    /// The table's columns are not known, so nothing can be checked.
    Unchecked,
    /// The columns are known and none (or several, differing only in case)
    /// match.
    Absent,
}

fn lookup(scope: &Scope<'_>, table: &str, column: &str) -> Lookup {
    let Some(columns) = scope.known.get(table).filter(|c| !c.is_empty()) else {
        return Lookup::Unchecked;
    };
    if let Some(exact) = columns.iter().find(|c| c.as_str() == column) {
        return Lookup::Found(exact.clone());
    }
    let folded: Vec<&String> = columns
        .iter()
        .filter(|c| c.eq_ignore_ascii_case(column))
        .collect();
    match folded.as_slice() {
        [one] => Lookup::Found((*one).clone()),
        _ => Lookup::Absent,
    }
}

/// `table.column` for a column that must be on `table`: checked when the
/// table's columns are known, taken as written when they are not.
fn on_table(scope: &Scope<'_>, table: &str, column: &str) -> Result<(String, String), String> {
    match lookup(scope, table, column) {
        Lookup::Found(real) => Ok((table.to_owned(), real)),
        Lookup::Unchecked => Ok((table.to_owned(), column.to_owned())),
        Lookup::Absent => Err(format!(
            "the column {} is not among the known columns of {}",
            short(column),
            short(table)
        )),
    }
}

/// The read a column of a CTE carries: its own sources and reasons, or the
/// fixed reason it is not one of the CTE's outputs (`DATA-57` D7).
fn read_from_cte(name: &str, columns: &[Out], column: &str) -> Result<Read, String> {
    // A CTE column matches exactly, case included (D3), unlike a table column
    // whose known spelling keeps today's single case-folded match.
    let matches: Vec<&Out> = columns.iter().filter(|o| o.name == column).collect();
    match matches.as_slice() {
        [one] => Ok(Read {
            sources: one.sources.clone(),
            reasons: one.reasons.clone(),
            transform: one.transform,
        }),
        [] => Err(format!(
            "the column {} is not a column of the CTE {}",
            short(column),
            short(name)
        )),
        _ => Err(format!(
            "the CTE {} has more than one column named {}",
            short(name),
            short(column)
        )),
    }
}

/// One identifier to the read it resolves to, or the reason it cannot be.
fn resolve(parts: &[Ident], scope: &Scope<'_>) -> Result<Read, String> {
    let Some((column, qualifier)) = parts.split_last() else {
        return Err("an empty identifier".to_owned());
    };
    let column = column.value.clone();
    if qualifier.is_empty() {
        resolve_unqualified(&column, scope)
    } else {
        resolve_qualified(&column, qualifier, scope)
    }
}

fn resolve_qualified(column: &str, qualifier: &[Ident], scope: &Scope<'_>) -> Result<Read, String> {
    let lowered: Vec<String> = qualifier
        .iter()
        .map(|i| i.value.to_ascii_lowercase())
        .collect();
    let raw = lowered.join(".");
    let keyed = canonicalize(&lowered);
    let hits: Vec<&Entry> = scope
        .entries
        .iter()
        .filter(|e| e.names.contains(&raw) || e.names.contains(&keyed))
        .collect();
    let [entry] = hits.as_slice() else {
        return Err(format!(
            "the qualifier {} of column {} matches {} in FROM",
            short(&raw),
            short(column),
            if hits.is_empty() {
                "no relation"
            } else {
                "several relations"
            }
        ));
    };
    match &entry.source {
        Source::Opaque(reason) => Err(reason.clone()),
        Source::Table(key) => on_table(scope, key, column).map(Read::table),
        Source::Cte { name, columns } => read_from_cte(name, columns, column),
    }
}

fn resolve_unqualified(column: &str, scope: &Scope<'_>) -> Result<Read, String> {
    // `ClickHouse` reads an unqualified name that is another item's alias as
    // that alias, so mapping it to a `FROM` column would be a wrong edge
    // (`DATA-57` D6). A qualified name and the item's own alias are not.
    if let Some((_, owner)) = scope.aliases.iter().find(|(name, _)| name == column)
        && *owner != scope.current_item
    {
        return Err(MSG_ALIAS_CAPTURE.to_owned());
    }
    // A source whose columns are not traced might hold this name too, so no
    // table can be named for it.
    let opaque = scope.entries.iter().find_map(|e| match &e.source {
        Source::Opaque(reason) => Some(reason),
        Source::Table(_) | Source::Cte { .. } => None,
    });
    if let Some(reason) = opaque {
        return Err(reason.clone());
    }
    // Keep the lone-table behaviour: one real table is the only possible
    // owner, checked by `on_table` rather than by a membership test. A lone
    // CTE likewise owns every read through it, with its own exact-name rule.
    let candidates: Vec<&Entry> = scope
        .entries
        .iter()
        .filter(|e| !matches!(e.source, Source::Opaque(_)))
        .collect();
    if let [entry] = candidates.as_slice() {
        match &entry.source {
            Source::Table(key) => return Ok(Read::table(on_table(scope, key, column)?)),
            Source::Cte { name, columns } => return read_from_cte(name, columns, column),
            Source::Opaque(_) => {}
        }
    }
    let mut holders: Vec<(&Entry, String)> = Vec::new();
    let mut unchecked: Vec<String> = Vec::new();
    for entry in &candidates {
        match &entry.source {
            Source::Opaque(_) => {}
            Source::Table(key) => match lookup(scope, key, column) {
                Lookup::Found(real) => holders.push((entry, real)),
                Lookup::Unchecked => unchecked.push(key.clone()),
                Lookup::Absent => {}
            },
            Source::Cte { name, columns } => {
                let matches: Vec<&Out> = columns.iter().filter(|o| o.name == column).collect();
                match matches.as_slice() {
                    [one] => holders.push((entry, one.name.clone())),
                    [] => {}
                    _ => {
                        return Err(format!(
                            "the CTE {} has more than one column named {}",
                            short(name),
                            short(column)
                        ));
                    }
                }
            }
        }
    }
    if !unchecked.is_empty() {
        // PR #106 review R1 BLOCKER 7: an unchecked table makes an unqualified column ambiguous
        return Err(format!(
            "the column {} is ambiguous: the columns of {} are not known; qualify it",
            short(column),
            unchecked
                .iter()
                .map(|k| short(k))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    match holders.as_slice() {
        [] => Err(format!("the column {} has no table in FROM", short(column))),
        [(entry, real)] => match &entry.source {
            Source::Table(key) => Ok(Read::table((key.clone(), real.clone()))),
            Source::Cte { name, columns } => read_from_cte(name, columns, real),
            Source::Opaque(reason) => Err(reason.clone()),
        },
        _ => Err(format!(
            "the column {} matches more than one table in FROM; qualify it",
            short(column)
        )),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use sqlparser::dialect::ClickHouseDialect;

    use super::*;

    fn known() -> HashMap<String, Vec<String>> {
        let table = |name: &str, columns: &[&str]| {
            (
                name.to_owned(),
                columns.iter().map(|c| (*c).to_owned()).collect(),
            )
        };
        HashMap::from([
            table("silver.orders", &["id", "Amount", "region"]),
            table("silver.users", &["id", "name"]),
            table("serving.target", &["a", "b"]),
        ])
    }

    fn run(sql: &str) -> Lineage {
        map_select(sql, &ClickHouseDialect {}, &known()).unwrap()
    }

    /// `(output, from_table.from_column, transform)` for each mapping.
    fn triples(lineage: &Lineage) -> Vec<(String, String, &'static str)> {
        lineage
            .mappings
            .iter()
            .map(|m| {
                (
                    m.output.clone(),
                    format!("{}.{}", m.from_table, m.from_column),
                    m.transform.as_str(),
                )
            })
            .collect()
    }

    fn t(output: &str, from: &str, transform: &'static str) -> (String, String, &'static str) {
        (output.to_owned(), from.to_owned(), transform)
    }

    fn reasons(lineage: &Lineage) -> Vec<(String, String)> {
        lineage
            .partial
            .iter()
            .map(|p| (p.output.clone(), p.reason.clone()))
            .collect()
    }

    #[test]
    fn a_bare_column_a_rename_a_cast_and_an_expression_are_told_apart() {
        let l = run(
            "SELECT id, amount AS amt, CAST(Amount AS Float64) AS f, Amount + 1 AS p \
             FROM silver.orders",
        );
        assert_eq!(
            triples(&l),
            vec![
                t("id", "silver.orders.id", "copy"),
                // The known column's spelling wins over the statement's.
                t("amt", "silver.orders.Amount", "copy"),
                t("f", "silver.orders.Amount", "cast"),
                t("p", "silver.orders.Amount", "expression"),
            ]
        );
        assert_eq!(l.partial, [] as [crate::sql_lineage::PartialOutput; 0]);
        assert_eq!(l.target, None);
    }

    #[test]
    fn a_double_colon_cast_is_a_cast_and_a_function_call_is_an_expression() {
        let l = run("SELECT id::String AS s, toInt32(id) AS k FROM silver.orders");
        assert_eq!(
            triples(&l),
            vec![
                t("s", "silver.orders.id", "cast"),
                t("k", "silver.orders.id", "expression"),
            ]
        );
    }

    #[test]
    fn a_join_resolves_through_aliases() {
        let l = run("SELECT o.id AS oid, u.name FROM silver.orders AS o \
             JOIN silver.users u ON o.id = u.id");
        assert_eq!(
            triples(&l),
            vec![
                t("oid", "silver.orders.id", "copy"),
                t("name", "silver.users.name", "copy"),
            ]
        );
    }

    #[test]
    fn an_unqualified_column_resolves_when_exactly_one_table_has_it() {
        let l = run("SELECT name, region FROM silver.orders o JOIN silver.users u ON o.id = u.id");
        assert_eq!(
            triples(&l),
            vec![
                t("name", "silver.users.name", "copy"),
                t("region", "silver.orders.region", "copy"),
            ]
        );
    }

    #[test]
    fn an_unqualified_column_in_two_tables_is_partial_not_guessed() {
        let l = run("SELECT id FROM silver.orders o JOIN silver.users u ON o.id = u.id");
        assert_eq!(l.mappings, [] as [crate::sql_lineage::Mapping; 0]);
        assert_eq!(
            reasons(&l),
            vec![(
                "id".to_owned(),
                "the column id matches more than one table in FROM; qualify it".to_owned()
            )]
        );
    }

    // PR #106 review R1 BLOCKER 7
    #[test]
    fn one_table_known_one_unknown_makes_an_unqualified_column_ambiguous() {
        let l = run("SELECT Amount FROM silver.orders o JOIN silver.unknown u ON o.id = u.id");
        assert_eq!(l.mappings, [] as [crate::sql_lineage::Mapping; 0]);
        assert_eq!(
            reasons(&l),
            vec![(
                "Amount".to_owned(),
                "the column Amount is ambiguous: the columns of silver.unknown are not known; qualify it".to_owned()
            )]
        );
    }

    #[test]
    fn an_unknown_qualifier_is_partial() {
        let l = run("SELECT x.id FROM silver.orders o");
        assert_eq!(l.mappings, [] as [crate::sql_lineage::Mapping; 0]);
        assert_eq!(
            reasons(&l),
            vec![(
                "id".to_owned(),
                "the qualifier x of column id matches no relation in FROM".to_owned()
            )]
        );
    }

    #[test]
    fn a_star_expands_from_known_columns() {
        let all = run("SELECT * FROM silver.orders");
        assert_eq!(
            triples(&all),
            vec![
                t("id", "silver.orders.id", "copy"),
                t("Amount", "silver.orders.Amount", "copy"),
                t("region", "silver.orders.region", "copy"),
            ]
        );
        assert_eq!(
            triples(&run("SELECT o.* FROM silver.orders o")),
            triples(&all)
        );
    }

    #[test]
    fn a_star_over_a_table_with_unknown_columns_is_partial() {
        let l = run("SELECT * FROM lake.bronze.raw");
        assert_eq!(l.mappings, [] as [crate::sql_lineage::Mapping; 0]);
        assert_eq!(
            reasons(&l),
            vec![(
                "*".to_owned(),
                "the columns of bronze.raw are not known".to_owned()
            )]
        );
    }

    #[test]
    fn a_star_with_options_is_partial() {
        let l = run("SELECT * EXCEPT (id) FROM silver.orders");
        assert_eq!(l.mappings, [] as [crate::sql_lineage::Mapping; 0]);
        assert_eq!(l.partial.len(), 1);
        assert_eq!(l.partial[0].reason, MSG_WILDCARD_OPTIONS);
    }

    #[test]
    fn a_star_over_a_join_marks_the_names_both_tables_share() {
        let l = run("SELECT * FROM silver.orders o JOIN silver.users u ON o.id = u.id");
        assert_eq!(
            triples(&l),
            vec![
                t("Amount", "silver.orders.Amount", "copy"),
                t("region", "silver.orders.region", "copy"),
                t("name", "silver.users.name", "copy"),
            ]
        );
        assert!(l.partial.iter().all(|p| p.output == "id"));
        assert_eq!(l.partial.len(), 2);
    }

    // DATA-57: the CTE half moved to `a_column_read_through_a_cte_names_the_table_behind_it`;
    // a subquery in `FROM` and a table function stay partial (D8).
    #[test]
    fn a_subquery_in_from_and_a_table_function_are_partial() {
        let sub = run("SELECT id FROM (SELECT id FROM silver.orders) s");
        assert_eq!(sub.partial[0].reason, MSG_SUBQUERY_FROM);
        let func = run("SELECT x FROM numbers(10)");
        assert_eq!(func.partial[0].reason, MSG_TABLE_FUNCTION);
        for l in [&sub, &func] {
            assert_eq!(l.mappings, [] as [crate::sql_lineage::Mapping; 0]);
        }
    }

    // DATA-57 T1: a `WITH` name resolves to the outputs of its query.

    #[test]
    fn a_column_read_through_a_cte_names_the_table_behind_it() {
        let l =
            run("WITH c AS (SELECT id, Amount AS amt FROM silver.orders) SELECT id, amt FROM c");
        assert_eq!(
            triples(&l),
            vec![
                t("id", "silver.orders.id", "copy"),
                t("amt", "silver.orders.Amount", "copy"),
            ]
        );
        assert_eq!(l.partial, [] as [crate::sql_lineage::PartialOutput; 0]);
    }

    #[test]
    fn a_star_over_a_cte_expands_to_its_columns() {
        let l = run("WITH c AS (SELECT * FROM silver.users) SELECT * FROM c");
        assert_eq!(
            triples(&l),
            vec![
                t("id", "silver.users.id", "copy"),
                t("name", "silver.users.name", "copy"),
            ]
        );
        assert_eq!(l.partial, [] as [crate::sql_lineage::PartialOutput; 0]);
    }

    #[test]
    fn a_nested_with_is_read_and_shadows_the_outer_name() {
        let l = run("WITH t AS (SELECT name FROM silver.users), \
             s AS (WITH t AS (SELECT id FROM silver.orders) SELECT id FROM t) \
             SELECT id FROM s");
        assert_eq!(triples(&l), vec![t("id", "silver.orders.id", "copy")]);
        assert_eq!(l.partial, [] as [crate::sql_lineage::PartialOutput; 0]);
    }

    #[test]
    fn a_cte_over_unknown_columns_leaves_the_star_partial() {
        let l = run("WITH c AS (SELECT * FROM silver.unknown) SELECT * FROM c");
        assert_eq!(l.mappings, [] as [crate::sql_lineage::Mapping; 0]);
        assert_eq!(
            reasons(&l),
            vec![(
                "*".to_owned(),
                "the columns of silver.unknown are not known".to_owned()
            )]
        );
    }

    #[test]
    fn a_union_inside_a_cte_is_carried_through_as_partial() {
        let l = run(
            "WITH c AS (SELECT id FROM silver.orders UNION ALL SELECT id FROM silver.users) \
             SELECT id FROM c",
        );
        assert_eq!(l.mappings, [] as [crate::sql_lineage::Mapping; 0]);
        assert_eq!(reasons(&l), vec![("id".to_owned(), MSG_UNION.to_owned())]);
    }

    #[test]
    fn a_subquery_in_from_inside_a_cte_is_carried_through_as_partial() {
        let l = run("WITH c AS (SELECT id FROM (SELECT id FROM silver.orders) s) SELECT id FROM c");
        assert_eq!(l.mappings, [] as [crate::sql_lineage::Mapping; 0]);
        assert_eq!(
            reasons(&l),
            vec![("id".to_owned(), MSG_SUBQUERY_FROM.to_owned())]
        );
    }

    #[test]
    fn a_recursive_with_is_partial() {
        let l = run("WITH RECURSIVE c AS (SELECT id FROM silver.orders) SELECT id FROM c");
        assert_eq!(l.mappings, [] as [crate::sql_lineage::Mapping; 0]);
        assert_eq!(
            reasons(&l),
            vec![("id".to_owned(), MSG_CTE_RECURSIVE.to_owned())]
        );
    }

    #[test]
    fn a_cte_with_a_column_list_is_partial() {
        let l = run("WITH c(x) AS (SELECT id FROM silver.orders) SELECT x FROM c");
        assert_eq!(l.mappings, [] as [crate::sql_lineage::Mapping; 0]);
        assert_eq!(
            reasons(&l),
            vec![("x".to_owned(), MSG_CTE_COLUMNS.to_owned())]
        );
    }

    #[test]
    fn a_name_read_before_its_cte_is_defined_is_partial() {
        let l = run(
            "WITH a AS (SELECT id FROM b), b AS (SELECT id FROM silver.orders) SELECT id FROM a",
        );
        assert_eq!(l.mappings, [] as [crate::sql_lineage::Mapping; 0]);
        assert_eq!(
            reasons(&l),
            vec![("id".to_owned(), MSG_CTE_FORWARD.to_owned())]
        );
    }

    #[test]
    fn a_name_that_differs_from_a_cte_only_in_case_is_partial() {
        let l = run("WITH c AS (SELECT id FROM silver.orders) SELECT id FROM C");
        assert_eq!(l.mappings, [] as [crate::sql_lineage::Mapping; 0]);
        assert_eq!(
            reasons(&l),
            vec![("id".to_owned(), MSG_CTE_CASE.to_owned())]
        );
    }

    #[test]
    fn a_column_the_cte_does_not_output_is_partial() {
        let l = run("WITH c AS (SELECT id FROM silver.orders) SELECT Amount FROM c");
        assert_eq!(l.mappings, [] as [crate::sql_lineage::Mapping; 0]);
        assert_eq!(
            reasons(&l),
            vec![(
                "Amount".to_owned(),
                "the column Amount is not a column of the CTE c".to_owned()
            )]
        );
    }

    #[test]
    fn a_cte_with_two_columns_of_one_name_is_partial_by_that_name() {
        let l = run(
            "WITH c AS (SELECT o.id, u.id FROM silver.orders AS o JOIN silver.users AS u \
             ON o.id = u.id) SELECT id FROM c",
        );
        assert_eq!(l.mappings, [] as [crate::sql_lineage::Mapping; 0]);
        assert_eq!(
            reasons(&l),
            vec![(
                "id".to_owned(),
                "the CTE c has more than one column named id".to_owned()
            )]
        );
    }

    #[test]
    fn an_unqualified_column_in_two_joined_ctes_is_partial() {
        let l = run("WITH a AS (SELECT id, name FROM silver.users), \
             b AS (SELECT id FROM silver.orders) \
             SELECT id FROM a JOIN b ON a.id = b.id");
        assert_eq!(l.mappings, [] as [crate::sql_lineage::Mapping; 0]);
        assert_eq!(
            reasons(&l),
            vec![(
                "id".to_owned(),
                "the column id matches more than one table in FROM; qualify it".to_owned()
            )]
        );
    }

    #[test]
    fn an_insert_through_a_with_maps_by_position() {
        let l = run(
            "INSERT INTO serving.target WITH c AS (SELECT id, name FROM silver.users) \
             SELECT id, name FROM c",
        );
        assert_eq!(l.target.as_deref(), Some("serving.target"));
        assert_eq!(
            triples(&l),
            vec![
                t("a", "silver.users.id", "copy"),
                t("b", "silver.users.name", "copy"),
            ]
        );
    }

    // DATA-57 T2: transforms compose through a WITH (D5).

    #[test]
    fn a_cast_of_a_cte_copy_is_a_cast_and_a_copy_of_a_cte_cast_is_a_cast() {
        let l = run(
            "WITH c AS (SELECT id, CAST(Amount AS Float64) AS f FROM silver.orders) \
             SELECT CAST(id AS String) AS s, f FROM c",
        );
        assert_eq!(
            triples(&l),
            vec![
                t("s", "silver.orders.id", "cast"),
                t("f", "silver.orders.Amount", "cast"),
            ]
        );
        assert_eq!(l.partial, [] as [crate::sql_lineage::PartialOutput; 0]);
    }

    #[test]
    fn a_cast_of_a_cte_cast_is_an_expression() {
        let l = run(
            "WITH c AS (SELECT CAST(Amount AS Float64) AS f FROM silver.orders) \
             SELECT CAST(f AS String) AS g FROM c",
        );
        assert_eq!(
            triples(&l),
            vec![t("g", "silver.orders.Amount", "expression")]
        );
    }

    #[test]
    fn a_copy_of_a_cte_constant_is_a_constant() {
        let l = run(
            "WITH c AS (SELECT 1 AS one, count() AS n FROM silver.orders) SELECT one, n FROM c",
        );
        assert_eq!(l.mappings, [] as [crate::sql_lineage::Mapping; 0]);
        assert_eq!(l.partial, [] as [crate::sql_lineage::PartialOutput; 0]);
        assert_eq!(l.constants, vec!["one".to_owned(), "n".to_owned()]);
    }

    #[test]
    fn the_compiled_shape_of_a_multi_step_pipeline_is_traced() {
        let l = run("WITH \
             `orders` AS (SELECT * FROM silver.`orders`), \
             `paid` AS (SELECT * FROM `orders` WHERE region = 'EU'), \
             `shaped` AS (SELECT id, CAST(Amount AS Float64) AS amount_f, upper(region) AS r FROM paid) \
             SELECT * FROM `shaped`");
        assert_eq!(
            triples(&l),
            vec![
                t("id", "silver.orders.id", "copy"),
                t("amount_f", "silver.orders.Amount", "cast"),
                t("r", "silver.orders.region", "expression"),
            ]
        );
        assert_eq!(l.partial, [] as [crate::sql_lineage::PartialOutput; 0]);
    }

    #[test]
    fn two_inputs_joined_in_a_sql_step_map_each_column_to_its_own_table() {
        let l = run("WITH \
             `orders` AS (SELECT * FROM silver.`orders`), \
             `users` AS (SELECT * FROM silver.`users`), \
             `joined` AS (SELECT o.id AS order_id, u.name, o.Amount * 2 AS doubled \
             FROM orders AS o JOIN users AS u ON o.id = u.id) \
             SELECT * FROM `joined`");
        assert_eq!(
            triples(&l),
            vec![
                t("order_id", "silver.orders.id", "copy"),
                t("name", "silver.users.name", "copy"),
                t("doubled", "silver.orders.Amount", "expression"),
            ]
        );
        assert_eq!(l.partial, [] as [crate::sql_lineage::PartialOutput; 0]);
    }

    // DATA-57 T3: an alias of another item captures an unqualified name (D6).

    #[test]
    fn an_alias_of_another_item_captures_an_unqualified_name() {
        let l = run("SELECT Amount AS id, id AS original FROM silver.orders");
        assert_eq!(triples(&l), vec![t("id", "silver.orders.Amount", "copy")]);
        assert_eq!(
            reasons(&l),
            vec![("original".to_owned(), MSG_ALIAS_CAPTURE.to_owned())]
        );
    }

    #[test]
    fn the_same_capture_inside_a_cte_is_carried_through() {
        let l = run(
            "WITH c AS (SELECT Amount AS id, id AS original FROM silver.orders) \
             SELECT original FROM c",
        );
        assert_eq!(l.mappings, [] as [crate::sql_lineage::Mapping; 0]);
        assert_eq!(
            reasons(&l),
            vec![("original".to_owned(), MSG_ALIAS_CAPTURE.to_owned())]
        );
    }

    #[test]
    fn an_items_own_alias_and_a_qualified_name_are_not_captured() {
        let l = run(
            "SELECT CAST(Amount AS Float64) AS Amount, o.id AS region_id, o.region AS id \
             FROM silver.orders AS o",
        );
        assert_eq!(
            triples(&l),
            vec![
                t("Amount", "silver.orders.Amount", "cast"),
                t("region_id", "silver.orders.id", "copy"),
                t("id", "silver.orders.region", "copy"),
            ]
        );
        assert_eq!(l.partial, [] as [crate::sql_lineage::PartialOutput; 0]);
    }

    #[test]
    fn id_as_id_does_not_capture() {
        let l = run("SELECT id AS id, id + 1 AS next FROM silver.orders");
        assert_eq!(
            triples(&l),
            vec![
                t("id", "silver.orders.id", "copy"),
                t("next", "silver.orders.id", "expression"),
            ]
        );
    }

    #[test]
    fn a_union_makes_every_output_of_the_left_branch_partial() {
        let l = run("SELECT id, name FROM silver.users UNION ALL SELECT id, id FROM silver.orders");
        assert_eq!(l.mappings, [] as [crate::sql_lineage::Mapping; 0]);
        assert_eq!(
            reasons(&l),
            vec![
                ("id".to_owned(), MSG_UNION.to_owned()),
                ("name".to_owned(), MSG_UNION.to_owned()),
            ]
        );
    }

    #[test]
    fn an_expression_with_a_subquery_is_partial_and_its_neighbours_are_not() {
        let l = run("SELECT (SELECT max(id) FROM silver.users) AS m, id FROM silver.orders");
        assert_eq!(triples(&l), vec![t("id", "silver.orders.id", "copy")]);
        assert_eq!(
            reasons(&l),
            vec![("m".to_owned(), MSG_SUBQUERY_EXPR.to_owned())]
        );
    }

    #[test]
    fn an_unaliased_expression_is_partial_and_a_constant_is_neither() {
        let l = run("SELECT sum(Amount), 1 AS one, now() AS at FROM silver.orders");
        assert_eq!(l.mappings, [] as [crate::sql_lineage::Mapping; 0]);
        assert_eq!(
            reasons(&l),
            vec![("sum(Amount)".to_owned(), MSG_UNNAMED.to_owned())]
        );
        assert_eq!(l.constants, vec!["one".to_owned(), "at".to_owned()]);
    }

    #[test]
    fn an_expression_over_two_columns_maps_each_of_them() {
        let l = run("SELECT Amount + id AS total FROM silver.orders");
        assert_eq!(
            triples(&l),
            vec![
                t("total", "silver.orders.Amount", "expression"),
                t("total", "silver.orders.id", "expression"),
            ]
        );
    }

    #[test]
    fn an_insert_without_a_column_list_maps_by_position_against_the_target() {
        let l = run("INSERT INTO serving.target SELECT id, name FROM silver.users");
        assert_eq!(l.target.as_deref(), Some("serving.target"));
        assert_eq!(
            triples(&l),
            vec![
                t("a", "silver.users.id", "copy"),
                t("b", "silver.users.name", "copy"),
            ]
        );
    }

    #[test]
    fn an_insert_with_a_column_list_lands_each_value_where_it_is_named() {
        let l = run("INSERT INTO serving.target (b, a) SELECT id, name FROM silver.users");
        assert_eq!(
            triples(&l),
            vec![
                t("b", "silver.users.id", "copy"),
                t("a", "silver.users.name", "copy"),
            ]
        );
    }

    #[test]
    fn a_positional_insert_into_a_table_with_unknown_columns_is_refused() {
        let err = map_select(
            "INSERT INTO serving.unknown SELECT id FROM silver.users",
            &ClickHouseDialect {},
            &known(),
        )
        .unwrap_err();
        assert!(err.0.contains("serving.unknown"), "{err}");
    }

    #[test]
    fn an_insert_whose_column_counts_differ_is_refused() {
        let err = map_select(
            "INSERT INTO serving.target SELECT id FROM silver.users",
            &ClickHouseDialect {},
            &known(),
        )
        .unwrap_err();
        assert!(err.0.contains("one to one"), "{err}");
    }

    #[test]
    fn a_create_table_as_select_names_its_target_and_uses_the_select_aliases() {
        let l = run(
            "CREATE TABLE IF NOT EXISTS serving.`t3` ENGINE = MergeTree ORDER BY tuple() \
             AS SELECT id AS k FROM silver.orders LIMIT 0",
        );
        assert_eq!(l.target.as_deref(), Some("serving.t3"));
        assert_eq!(triples(&l), vec![t("k", "silver.orders.id", "copy")]);
    }

    #[test]
    fn statements_that_are_not_a_select_or_do_not_parse_are_unmappable() {
        for sql in [
            "DELETE FROM silver.orders WHERE id = 1",
            "SELECT 1; SELECT 2",
            "SELECT FROM",
        ] {
            assert!(
                map_select(sql, &ClickHouseDialect {}, &known()).is_err(),
                "{sql}"
            );
        }
    }

    #[test]
    fn a_long_identifier_cannot_make_a_reason_longer_than_a_row_allows() {
        let long = "c".repeat(500);
        let l = run(&format!(
            "SELECT {long} FROM silver.orders o JOIN silver.users u ON o.id = u.id"
        ));
        assert_eq!(l.partial.len(), 1);
        assert!(l.partial[0].reason.chars().count() <= 300);
    }
}
