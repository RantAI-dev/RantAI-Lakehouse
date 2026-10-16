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
//! * a column read through a CTE, a subquery in `FROM` or a table function;
//! * a `UNION` (every output of the left branch is partial);
//! * an unqualified identifier that matches no table, or several, in `FROM`,
//!   or that could belong to a source whose columns are not known;
//! * an expression holding a subquery, or an unaliased expression (the engine
//!   names it, so its column name is not known here).
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
}

/// One output slot, before it is split into mappings and partials.
struct Out {
    name: String,
    sources: Vec<(String, String)>,
    reasons: Vec<String>,
    transform: Transform,
    /// A `*` that could not be expanded: one slot standing for an unknown
    /// number of columns, so a positional `INSERT` cannot be read.
    unexpanded_star: bool,
}

const MSG_SUBQUERY_EXPR: &str = "the expression contains a subquery, which is not traced";
const MSG_UNNAMED: &str =
    "the expression has no alias, so the engine chooses the column name; add an alias";
const MSG_UNION: &str = "built with UNION, INTERSECT or EXCEPT; the branches are not traced";
const MSG_SUBQUERY_FROM: &str = "read through a subquery in FROM, which is not traced";
const MSG_TABLE_FUNCTION: &str = "read through a table function or another relation that is not a plain table, which is not traced";
const MSG_WILDCARD_OPTIONS: &str =
    "the * has EXCEPT, REPLACE, RENAME or similar options, which are not traced";

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
    let ctes: Vec<String> = query
        .with
        .iter()
        .flat_map(|with| with.cte_tables.iter())
        .map(|cte| cte.alias.name.value.to_ascii_lowercase())
        .collect();
    let mut outs = outputs_of_body(&query.body, &ctes, known_columns)?;

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

/// The outputs of a query body: a `SELECT`, a parenthesized query, or a set
/// operation (whose outputs are all partial).
fn outputs_of_body(
    body: &SetExpr,
    ctes: &[String],
    known: &HashMap<String, Vec<String>>,
) -> Result<Vec<Out>, Unmappable> {
    match body {
        SetExpr::Select(select) => {
            let scope = Scope {
                entries: select_entries(&select.from, ctes),
                known,
            };
            let mut outs = Vec::new();
            for item in &select.projection {
                outs.extend(outputs_of_item(item, &scope));
            }
            Ok(outs)
        }
        SetExpr::Query(inner) => {
            let mut inner_ctes = ctes.to_vec();
            inner_ctes.extend(
                inner
                    .with
                    .iter()
                    .flat_map(|with| with.cte_tables.iter())
                    .map(|cte| cte.alias.name.value.to_ascii_lowercase()),
            );
            outputs_of_body(&inner.body, &inner_ctes, known)
        }
        SetExpr::SetOperation { left, .. } => {
            let mut outs = outputs_of_body(left, ctes, known)?;
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

/// Every relation of a `FROM` clause with the names it can be qualified by.
fn select_entries(from: &[TableWithJoins], ctes: &[String]) -> Vec<Entry> {
    let mut entries = Vec::new();
    for twj in from {
        push_factor(&twj.relation, ctes, &mut entries);
        for join in &twj.joins {
            push_factor(&join.relation, ctes, &mut entries);
        }
    }
    entries
}

fn push_factor(factor: &TableFactor, ctes: &[String], entries: &mut Vec<Entry>) {
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
            if name.0.len() == 1 && ctes.contains(&key) {
                entries.push(Entry {
                    names,
                    source: Source::Opaque(format!(
                        "read through the CTE {}, which is not traced",
                        short(&key)
                    )),
                });
                return;
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
            push_factor(&table_with_joins.relation, ctes, entries);
            for join in &table_with_joins.joins {
                push_factor(&join.relation, ctes, entries);
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
    // Two tables with a column of the same name: the engine renames one, and
    // which one is not decided here.
    if entries.len() > 1 {
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
    for parts in &reads.identifiers {
        match resolve(parts, scope) {
            Ok(pair) => {
                if !out.sources.contains(&pair) {
                    out.sources.push(pair);
                }
            }
            Err(reason) => {
                if !out.reasons.contains(&reason) {
                    out.reasons.push(reason);
                }
            }
        }
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

/// One identifier to `(table key, column)`, or the reason it cannot be.
fn resolve(parts: &[Ident], scope: &Scope<'_>) -> Result<(String, String), String> {
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

fn resolve_qualified(
    column: &str,
    qualifier: &[Ident],
    scope: &Scope<'_>,
) -> Result<(String, String), String> {
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
        Source::Table(key) => on_table(scope, key, column),
    }
}

fn resolve_unqualified(column: &str, scope: &Scope<'_>) -> Result<(String, String), String> {
    // A source whose columns are not traced might hold this name too, so no
    // table can be named for it.
    let opaque = scope.entries.iter().find_map(|e| match &e.source {
        Source::Opaque(reason) => Some(reason),
        Source::Table(_) => None,
    });
    if let Some(reason) = opaque {
        return Err(reason.clone());
    }
    let tables: Vec<&String> = scope
        .entries
        .iter()
        .filter_map(|e| match &e.source {
            Source::Table(key) => Some(key),
            Source::Opaque(_) => None,
        })
        .collect();
    match tables.as_slice() {
        [] => Err(format!("the column {} has no table in FROM", short(column))),
        [only] => on_table(scope, only, column),
        several => {
            let holders: Vec<(&String, String)> = several
                .iter()
                .filter_map(|key| match lookup(scope, key, column) {
                    Lookup::Found(real) => Some((*key, real)),
                    Lookup::Unchecked | Lookup::Absent => None,
                })
                .collect();
            match holders.as_slice() {
                [(key, real)] => Ok(((*key).clone(), real.clone())),
                [] => Err(format!(
                    "the column {} matches no table in FROM whose columns are known",
                    short(column)
                )),
                _ => Err(format!(
                    "the column {} matches more than one table in FROM; qualify it",
                    short(column)
                )),
            }
        }
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
        assert!(l.partial.is_empty());
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
        assert!(l.mappings.is_empty());
        assert_eq!(
            reasons(&l),
            vec![(
                "id".to_owned(),
                "the column id matches more than one table in FROM; qualify it".to_owned()
            )]
        );
    }

    #[test]
    fn an_unknown_qualifier_is_partial() {
        let l = run("SELECT x.id FROM silver.orders o");
        assert!(l.mappings.is_empty());
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
        assert!(l.mappings.is_empty());
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
        assert!(l.mappings.is_empty());
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

    #[test]
    fn a_cte_a_subquery_in_from_and_a_table_function_are_partial() {
        let cte = run("WITH c AS (SELECT id FROM silver.orders) SELECT id FROM c");
        assert_eq!(
            reasons(&cte),
            vec![(
                "id".to_owned(),
                "read through the CTE c, which is not traced".to_owned()
            )]
        );
        let sub = run("SELECT id FROM (SELECT id FROM silver.orders) s");
        assert_eq!(sub.partial[0].reason, MSG_SUBQUERY_FROM);
        let func = run("SELECT x FROM numbers(10)");
        assert_eq!(func.partial[0].reason, MSG_TABLE_FUNCTION);
        for l in [&cte, &sub, &func] {
            assert!(l.mappings.is_empty());
        }
    }

    #[test]
    fn a_union_makes_every_output_of_the_left_branch_partial() {
        let l = run("SELECT id, name FROM silver.users UNION ALL SELECT id, id FROM silver.orders");
        assert!(l.mappings.is_empty());
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
        assert!(l.mappings.is_empty());
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
