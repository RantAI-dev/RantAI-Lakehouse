//! The fixed, injection-safe transform vocabulary an authored pipeline's
//! `transforms` column (migration 0036) may contain. Never executes a
//! caller-authored string as SQL directly — `filter(expr)`'s `expr` is
//! parsed into a small grammar (one column identifier, one operator from a
//! fixed set, one literal) and re-rendered through
//! [`lakehouse_core::ident::Ident`]/[`lakehouse_core::ident::SqlLiteral`],
//! never string-concatenated from the caller's own text. `POST
//! /api/pipelines` calls [`parse_transform`] on every entry in
//! `CreatePipelineBody.transforms` and returns 400 on the first failure —
//! an unparseable transform is never stored, let alone executed.
//!
//! The vocabulary is intentionally small (`dedupe`, `filter`, `rename`,
//! `cast`, `select`) so a later Python port (driven by a shared test-vector
//! file) can check the identical set of payloads against an independent
//! implementation.

use lakehouse_core::ident::{Ident, IdentError, SqlLiteral};

/// Reasons [`parse_transform`] refused an input string. Every variant
/// names the offending FIELD, never the caller's payload — the payload is
/// exactly the untrusted text this module exists to keep out of an error
/// message that might itself be logged or echoed back.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TransformError {
    /// The transform did not start with a known verb.
    #[error("unknown transform verb")]
    UnknownVerb,
    /// An argument in a `dedupe`/`rename`/`cast`/`select` position was not
    /// a valid [`Ident`].
    #[error("invalid identifier")]
    InvalidIdentifier,
    /// `filter(expr)`'s `expr` did not match `<ident> <op> '<literal>'`.
    #[error("invalid filter expression")]
    InvalidFilter,
    /// `cast(col,type)`'s `type` was not in the allowlist.
    #[error("cast type not allowed")]
    DisallowedCastType,
    /// The transform's argument list did not match its verb's arity/shape.
    #[error("malformed transform")]
    Malformed,
}

/// The only `cast(col,type)` targets accepted — `ClickHouse` type names a
/// dedicated Bronze/Silver transform legitimately needs, nothing else (no
/// `AggregateFunction`, no parametrized/nested types, which would reopen
/// an injection surface through the type-name slot itself).
const ALLOWED_CAST_TYPES: [&str; 8] = [
    "String", "Int32", "Int64", "Float64", "Boolean", "Date", "DateTime", "UUID",
];

/// `filter(expr)`'s only permitted comparison operators. No `<>` (use
/// `!=`), no `LIKE`/`IN`/boolean connectives — those would reopen the
/// subquery/function-call injection surface the grammar exists to close.
const ALLOWED_FILTER_OPERATORS: [&str; 6] = ["!=", "<=", ">=", "=", "<", ">"];

/// One parsed, injection-safe transform step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transform {
    /// `dedupe(key)` — keep the first row per `key`.
    Dedupe {
        /// The deduplication key column.
        key: Ident,
    },
    /// `filter(column op 'literal')` — keep rows matching one comparison.
    Filter {
        /// The column being compared.
        column: Ident,
        /// One of [`ALLOWED_FILTER_OPERATORS`].
        operator: &'static str,
        /// The right-hand-side literal, self-escaping on render.
        literal: SqlLiteral,
    },
    /// `rename(from,to)` — rename one column.
    Rename {
        /// The existing column name.
        from: Ident,
        /// The new column name.
        to: Ident,
    },
    /// `cast(col,type)` — cast one column to an allowlisted type.
    Cast {
        /// The column being cast.
        column: Ident,
        /// One of [`ALLOWED_CAST_TYPES`].
        target_type: &'static str,
    },
    /// `select(cols)` — project down to the named columns.
    Select {
        /// The columns to keep, in the caller's order.
        columns: Vec<Ident>,
    },
}

impl Transform {
    /// Render as a `ClickHouse` SQL fragment. Every dynamic piece goes
    /// through [`Ident`]'s or [`SqlLiteral`]'s own `Display`
    /// (identifier-safety / literal-escaping), never raw interpolation of
    /// caller text.
    ///
    /// Not called from Rust at runtime yet — it is the reference twin of
    /// `dagster/dispar_orchestrate/authored_transforms.py::render_clickhouse`
    /// (Phase E2), which is what actually renders a `Transform` into the
    /// `ClickHouse` SQL `authored_factory.py`'s generated jobs execute.
    /// `WS4` item D3/D4/G2/G3 (this commit) is the first task to add a
    /// second `mod transform_grammar;` declaration for this file (in
    /// `main.rs`, alongside `lib.rs`'s existing `pub mod`) so
    /// `routes::pipelines::create`'s new `parse_transform` call compiles
    /// for the `lakehouse-api` BINARY target, not only its library target
    /// — that surfaces `clippy::dead_code` here for the first time: a
    /// binary crate has no public-API boundary keeping an unused `pub fn`
    /// alive the way the library target's `pub mod` already did.
    #[allow(
        dead_code,
        reason = "reference twin of authored_transforms.py::render_clickhouse (Phase E2); \
                  exercised by this file's own tests, not yet called from Rust at runtime"
    )]
    #[must_use]
    pub fn render_clickhouse(&self) -> String {
        match self {
            Self::Dedupe { key } => format!("ORDER BY {key}"),
            Self::Filter {
                column,
                operator,
                literal,
            } => format!("{column} {operator} {literal}"),
            Self::Rename { from, to } => format!("{from} AS {to}"),
            Self::Cast {
                column,
                target_type,
            } => format!("CAST({column} AS {target_type})"),
            Self::Select { columns } => columns
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", "),
        }
    }
}

/// Parse one transform-vocabulary string. See the module doc comment.
///
/// # Errors
///
/// [`TransformError`] on any string outside the fixed grammar — an unknown
/// verb, a malformed argument list, an identifier that fails
/// [`Ident::new`], a filter expression outside `<ident> <op> 'literal'`
/// with `op` in [`ALLOWED_FILTER_OPERATORS`], or a cast type outside
/// [`ALLOWED_CAST_TYPES`].
pub fn parse_transform(input: &str) -> Result<Transform, TransformError> {
    let input = input.trim();
    let (verb, rest) = input.split_once('(').ok_or(TransformError::Malformed)?;
    let args = rest.strip_suffix(')').ok_or(TransformError::Malformed)?;
    match verb {
        "dedupe" => Ok(Transform::Dedupe {
            key: parse_ident(args)?,
        }),
        "filter" => parse_filter(args),
        "rename" => {
            let (a, b) = args.split_once(',').ok_or(TransformError::Malformed)?;
            Ok(Transform::Rename {
                from: parse_ident(a)?,
                to: parse_ident(b)?,
            })
        }
        "cast" => {
            let (col, ty) = args.split_once(',').ok_or(TransformError::Malformed)?;
            let ty = ty.trim();
            let allowed = ALLOWED_CAST_TYPES
                .iter()
                .find(|t| **t == ty)
                .ok_or(TransformError::DisallowedCastType)?;
            Ok(Transform::Cast {
                column: parse_ident(col)?,
                target_type: allowed,
            })
        }
        "select" => {
            let columns = args
                .split(',')
                .map(parse_ident)
                .collect::<Result<Vec<_>, _>>()?;
            if columns.is_empty() {
                return Err(TransformError::Malformed);
            }
            Ok(Transform::Select { columns })
        }
        _ => Err(TransformError::UnknownVerb),
    }
}

fn parse_ident(s: &str) -> Result<Ident, TransformError> {
    Ident::new(s.trim()).map_err(|_: IdentError| TransformError::InvalidIdentifier)
}

/// `filter(expr)`'s grammar: EXACTLY `<identifier> <op> '<literal>'` — one
/// column, one operator from [`ALLOWED_FILTER_OPERATORS`], one
/// single-quoted literal. No boolean connectives (`AND`/`OR`), no
/// subqueries, no function calls, no comments — a caller needing anything
/// richer than one comparison is out of scope for this vocabulary;
/// multiple `filter(...)` entries in `transforms` compose as an implicit
/// `AND` at the pipeline level instead.
///
/// Operators are tried longest-first (`ALLOWED_FILTER_OPERATORS` is
/// ordered `!=, <=, >=, =, <, >`) so `<=`/`>=`/`!=` are never mis-split on
/// their leading `<`/`>`/prefix character.
fn parse_filter(expr: &str) -> Result<Transform, TransformError> {
    let expr = expr.trim();
    for op in ALLOWED_FILTER_OPERATORS {
        let Some((col, val)) = expr.split_once(op) else {
            continue;
        };
        let col = col.trim();
        let val = val.trim();
        if !val.starts_with('\'') || !val.ends_with('\'') || val.len() < 2 {
            continue;
        }
        let literal_body = &val[1..val.len() - 1];
        // Reject anything a single-quoted literal has no business
        // containing: another quote (would need doubling, which this
        // minimal grammar does not support — reject rather than guess), a
        // semicolon, parentheses (blocks function-call/subquery injection
        // outright, since a bare literal never needs them), or SQL
        // comment markers.
        if literal_body.contains(['\'', ';', '(', ')'])
            || literal_body.contains("--")
            || literal_body.contains("/*")
        {
            return Err(TransformError::InvalidFilter);
        }
        let column = parse_ident(col)?;
        return Ok(Transform::Filter {
            column,
            operator: op,
            literal: SqlLiteral::from(literal_body),
        });
    }
    Err(TransformError::InvalidFilter)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    // ── Happy path, one per verb ─────────────────────────────────────
    #[test]
    fn parses_dedupe_with_a_valid_identifier() {
        assert!(
            matches!(parse_transform("dedupe(order_id)"), Ok(Transform::Dedupe { key }) if key.as_str() == "order_id")
        );
    }
    #[test]
    fn parses_filter_with_a_valid_comparison() {
        let t = parse_transform("filter(status = 'active')").unwrap();
        assert!(matches!(t, Transform::Filter { .. }));
    }
    #[test]
    fn parses_filter_with_a_two_character_operator() {
        assert!(matches!(
            parse_transform("filter(amount <= '100')"),
            Ok(Transform::Filter { operator: "<=", .. })
        ));
        assert!(matches!(
            parse_transform("filter(amount != '0')"),
            Ok(Transform::Filter { operator: "!=", .. })
        ));
    }
    #[test]
    fn parses_rename() {
        assert!(matches!(
            parse_transform("rename(old_col,new_col)"),
            Ok(Transform::Rename { .. })
        ));
    }
    #[test]
    fn parses_cast_with_an_allowlisted_type() {
        assert!(matches!(
            parse_transform("cast(amount,Int64)"),
            Ok(Transform::Cast { .. })
        ));
    }
    #[test]
    fn parses_select_with_a_column_list() {
        assert!(matches!(
            parse_transform("select(id,name,amount)"),
            Ok(Transform::Select { .. })
        ));
    }

    // ── Injection classes named in the brief, each its own test ──────
    #[test]
    fn rejects_stacked_statement_injection() {
        assert!(parse_transform("filter(1=1; DROP TABLE x)").is_err());
    }
    #[test]
    fn rejects_subquery_injection() {
        assert!(parse_transform("filter(id IN (SELECT id FROM other_table))").is_err());
    }
    #[test]
    fn rejects_function_call_injection() {
        assert!(parse_transform("filter(sleep(status))").is_err());
    }
    #[test]
    fn rejects_sql_comment_injection() {
        assert!(parse_transform("filter(status = 'a' -- ')").is_err());
        assert!(parse_transform("filter(status = 'a' /* */)").is_err());
    }
    #[test]
    fn rejects_an_operator_outside_the_fixed_set() {
        assert!(parse_transform("filter(status <> 'x')").is_err()); // only =, !=, <, <=, >, >= allowed
    }
    #[test]
    fn rejects_a_non_allowlisted_cast_type() {
        assert!(parse_transform("cast(col,UDF_EVIL_TYPE)").is_err());
    }
    #[test]
    fn rejects_an_invalid_identifier_in_dedupe() {
        assert!(parse_transform("dedupe(order_id; DROP TABLE x)").is_err());
    }
    #[test]
    fn rejects_an_unknown_verb() {
        assert!(parse_transform("exec(rm -rf /)").is_err());
    }
    #[test]
    fn renders_filter_to_bound_clickhouse_sql_literal_form() {
        let t = parse_transform("filter(status = 'active')").unwrap();
        let rendered = t.render_clickhouse();
        // Literal is rendered through SqlLiteral's own escaping, not string
        // concatenation — a literal containing a quote is escaped, not a
        // syntax break.
        assert_eq!(rendered, "status = 'active'");
    }
    #[test]
    fn renders_a_literal_containing_a_quote_safely() {
        // The raw quote in the source is refused outright (see
        // rejects_a_backslash_quote_bypass_payload below) — this test
        // documents the OUTCOME of that refusal: no rendered fragment ever
        // contains an unescaped `' OR '1'='1` tautology.
        let result = parse_transform("filter(name = 'o''brien')");
        assert!(result.is_err());
    }

    // ── Judge review V7: backslash-quote bypass ───────────────────────
    #[test]
    fn rejects_a_backslash_quote_bypass_payload() {
        // `\' OR '1'='1` as the literal body contains a raw `'`, which the
        // existing quote-character ban above already refuses — this test
        // documents that the classic backslash-quote bypass class cannot
        // even reach `SqlLiteral`'s escaping.
        assert!(parse_transform(r"filter(name = '\' OR '1'='1')").is_err());
    }
    #[test]
    fn renders_a_legitimate_backslash_in_a_literal_safely() {
        // A bare backslash with NO embedded quote is accepted (nothing in
        // the ban list forbids `\` alone) and must render with the
        // backslash doubled by `SqlLiteral`'s own `Display` (backslashes
        // doubled BEFORE quotes), never passed through raw.
        let t = parse_transform(r"filter(path = 'C:\data')").unwrap();
        assert_eq!(t.render_clickhouse(), r"path = 'C:\\data'");
    }

    // ── Field-naming, non-echoing error messages ──────────────────────
    #[test]
    fn refusal_names_the_field_not_the_payload() {
        let payload = "'; DROP TABLE pipeline_definition; --";
        let err = parse_transform(&format!("dedupe({payload})")).unwrap_err();
        let message = err.to_string();
        assert!(
            !message.contains(payload),
            "error message must not echo the payload: {message}"
        );
        assert_eq!(err, TransformError::InvalidIdentifier);
    }
}
