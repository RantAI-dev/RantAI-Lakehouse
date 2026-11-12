//! BLD-1 part 1 / D2 and D4: turn a stored pipeline definition — named
//! inputs, ordered named steps — into the ONE `SELECT` the orchestrator runs.
//!
//! The compiler lives in the API, not in `Dagster`, for the same reason the
//! policy rewrite does (BLD-1 / D4): the answer has to be the same at save
//! time (D9), at preview time (S2) and at run time (D8), and a second
//! implementation in the orchestrator would be free to drift from it. The
//! orchestrator now asks this module's answer through
//! `POST /api/pipelines/runnable/{id}/compile` and runs exactly what comes
//! back.
//!
//! # The shape of the output
//!
//! One `WITH` clause names every input and every step, and the last line
//! selects the last step (or the only input, for a plain copy):
//!
//! ```sql
//! WITH
//!   `orders` AS (SELECT * FROM icecat_api.`bronze.orders`),
//!   `paid` AS (SELECT * FROM `orders` WHERE status = 'paid'),
//!   `by_country` AS (
//! SELECT c.country, sum(p.amount) AS revenue FROM paid AS p ...
//! )
//! SELECT * FROM `by_country`
//! ```
//!
//! A `transform` step is rendered by [`render_transform_step`], a port of
//! the orchestrator's former `_build_select_sql` over
//! [`crate::transform_grammar::Transform::render_clickhouse`]. A SQL step's
//! text goes in as written, between its own line breaks, so a `--` comment
//! at its end cannot swallow the closing bracket.
//!
//! # What is refused, and how
//!
//! Every refusal is a [`DefinitionError`] naming the input or step at fault
//! (`inputs[2]`, `steps[0]`, `steps[1].transforms[0]`) followed by fixed text
//! written in this repository — never the parser's, the rewriter's or
//! `ClickHouse`'s own words (D7, AGENTS.md rule 4). The route turns it into a
//! 400; the gate and the rewrite that run after it turn their own refusals
//! into a 422.
//!
//! The rules, in the order they are applied (so the cheapest refusal wins):
//!
//! 1. 1 to 5 inputs and at most 20 steps (D2, decisions 4 and 15).
//! 2. Every name is an `Ident` of at most 48 characters (D2), not starting
//!    with `__`, and unique across inputs and steps without regard to case.
//! 3. Every zone and table is an `Ident` (F2: the API checks the names it
//!    stores, instead of leaving it to the orchestrator at run time).
//! 4. A `transform` step reads an input or an earlier step, and each of its
//!    transform strings parses (D2, D4).
//! 5. A SQL step is one read-only statement (D5, [`check_pipeline_step_sql`]),
//!    and every name it reads is an input or an earlier step, EXACTLY — case
//!    included, with a qualified `zone.table` refused (D6, R1-B6 / D6a).
//! 6. The output table is not one of the inputs (D2).

use lakehouse_core::ident::Ident;
use lakehouse_store::pipelines::{PipelineInput, PipelineStep};

use crate::sql_guard::check_pipeline_step_sql;
use crate::transform_grammar::{Transform, parse_transform};

/// Tokens a step's SQL may not contain, already enforced by
/// [`check_pipeline_step_sql`]; named here so the compiler's own tests read
/// as the rule they are.
///
/// At most 5 inputs (decision 4).
pub const MAX_INPUTS: usize = 5;
/// At most 20 steps (decision 15).
pub const MAX_STEPS: usize = 20;
/// Longest name, in characters (D2).
pub const MAX_NAME_CHARS: usize = 48;

/// One refusal from the compiler: which input, step or transform is at
/// fault, and fixed text saying why.
///
/// `Display` is the wire message the console parses: it starts with the
/// path (`steps[1]: ...`), the shape the page already understands for
/// `transforms[0]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionError {
    path: String,
    message: &'static str,
}

impl DefinitionError {
    fn at(path: String, message: &'static str) -> Self {
        Self { path, message }
    }

    fn input(index: usize, message: &'static str) -> Self {
        Self::at(format!("inputs[{index}]"), message)
    }

    fn step(index: usize, message: &'static str) -> Self {
        Self::at(format!("steps[{index}]"), message)
    }

    /// The `inputs[i]` / `steps[i]` prefix, for a caller that highlights the
    /// item in an editor.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The fixed explanation, without the path prefix.
    #[must_use]
    pub fn message(&self) -> &'static str {
        self.message
    }
}

impl std::fmt::Display for DefinitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

// ── the fixed sentences ────────────────────────────────────────────────
const NO_INPUT: &str = "a pipeline needs at least one input";
const TOO_MANY_INPUTS: &str = "a pipeline may have at most 5 inputs";
const TOO_MANY_STEPS: &str = "a pipeline may have at most 20 steps";
const INVALID_NAME: &str = "a name must be letters, digits and underscores (not starting with a digit or __), \
     at most 48 characters";
const DUPLICATE_NAME: &str = "this name is already used by another input or step";
const INVALID_TABLE: &str =
    "a zone and a table must be plain names (letters, digits and underscores)";
const READS_NOT_INPUT: &str = "this step reads a table that is not an input; add it under Inputs";
const READS_LATER_STEP: &str = "this step reads a step that comes after it; reorder the steps";
const TRANSFORM_REFUSED: &str = "this transform is not in the supported grammar";
const OUTPUT_IS_INPUT: &str =
    "the output table is one of the inputs; the output cannot be one of the inputs";
const BRONZE_NEEDS_CATALOG: &str =
    "a bronze input is read through the catalog database, which this deployment has not configured";
const UP_TO_UNKNOWN: &str = "upTo must name an input or a step of this pipeline";

/// Whether `name` is one D2 allows for an input or a step.
fn valid_name(name: &str) -> bool {
    name.chars().count() <= MAX_NAME_CHARS && !name.starts_with("__") && Ident::new(name).is_ok()
}

/// Lowercased, for the one comparison D2 makes without regard to case:
/// whether two names collide. Every other comparison is exact (R1-B6 / D6a).
fn key(name: &str) -> String {
    name.to_ascii_lowercase()
}

/// Check the definition and compile it into one statement, stopping after
/// `up_to` (an input or step name) when given — the preview's seam (S2).
///
/// `catalog_db` is `Config::iceberg_query_db`: the database a Bronze input is
/// read through, exactly as the orchestrator names it. `None` refuses a
/// Bronze input rather than naming a database that does not exist.
///
/// # Errors
///
/// [`DefinitionError`] naming the input or step at fault for any rule in the
/// module doc.
pub fn compile(
    inputs: &[PipelineInput],
    steps: &[PipelineStep],
    up_to: Option<&str>,
    catalog_db: Option<&str>,
) -> Result<String, DefinitionError> {
    let (limit, last) = validate(inputs, steps, up_to)?;
    let mut ctes: Vec<String> = Vec::with_capacity(inputs.len() + limit);
    for (index, input) in inputs.iter().enumerate() {
        ctes.push(format!(
            "  `{}` AS (SELECT * FROM {})",
            input.name,
            input_source(input, index, catalog_db)?
        ));
    }
    for (index, step) in steps[..limit].iter().enumerate() {
        ctes.push(render_step(step, index)?);
    }
    Ok(format!(
        "WITH\n{}\nSELECT * FROM `{}`",
        ctes.join(",\n"),
        last
    ))
}

/// Check the whole definition and compile all of it, refusing an output
/// table that is one of the inputs — the save path's entry point (D9).
///
/// `target` is the pipeline's output (`zone`, `table`).
///
/// # Errors
///
/// [`DefinitionError`] naming the input or step at fault, exactly as
/// [`compile`].
pub fn validate_and_compile(
    inputs: &[PipelineInput],
    steps: &[PipelineStep],
    target: Option<(&str, &str)>,
    catalog_db: Option<&str>,
) -> Result<String, DefinitionError> {
    let compiled = compile(inputs, steps, None, catalog_db)?;
    if let Some((zone, table)) = target {
        for (index, input) in inputs.iter().enumerate() {
            if input.zone == zone && input.table == table {
                return Err(DefinitionError::input(index, OUTPUT_IS_INPUT));
            }
        }
    }
    Ok(compiled)
}

/// Every rule up to the point the compiler needs an answer from: returns how
/// many steps to render, and the name the final `SELECT` reads.
fn validate(
    inputs: &[PipelineInput],
    steps: &[PipelineStep],
    up_to: Option<&str>,
) -> Result<(usize, String), DefinitionError> {
    if inputs.is_empty() {
        return Err(DefinitionError::at("inputs".to_owned(), NO_INPUT));
    }
    if inputs.len() > MAX_INPUTS {
        return Err(DefinitionError::input(MAX_INPUTS, TOO_MANY_INPUTS));
    }
    if steps.len() > MAX_STEPS {
        return Err(DefinitionError::step(MAX_STEPS, TOO_MANY_STEPS));
    }

    // Names first, across the whole definition: uniqueness is a property of
    // the list, not of a prefix, and the preview has to know which name a
    // later step already took.
    let mut seen: Vec<String> = Vec::with_capacity(inputs.len() + steps.len());
    for (index, input) in inputs.iter().enumerate() {
        if !valid_name(&input.name) {
            return Err(DefinitionError::input(index, INVALID_NAME));
        }
        let key = key(&input.name);
        if seen.contains(&key) {
            return Err(DefinitionError::input(index, DUPLICATE_NAME));
        }
        seen.push(key);
    }
    for (index, step) in steps.iter().enumerate() {
        if !valid_name(step.name()) {
            return Err(DefinitionError::step(index, INVALID_NAME));
        }
        let key = key(step.name());
        if seen.contains(&key) {
            return Err(DefinitionError::step(index, DUPLICATE_NAME));
        }
        seen.push(key);
    }

    // F2: the API checks the zone and table names it stores.
    for (index, input) in inputs.iter().enumerate() {
        if Ident::new(input.zone.clone()).is_err() || Ident::new(input.table.clone()).is_err() {
            return Err(DefinitionError::input(index, INVALID_TABLE));
        }
    }

    // The prefix to compile: everything, or up to the named item.
    let (limit, last) = match up_to {
        None => (
            steps.len(),
            steps
                .last()
                .map_or_else(|| inputs[0].name.clone(), |step| step.name().to_owned()),
        ),
        Some(name) => {
            if let Some(index) = inputs.iter().position(|i| i.name == name) {
                // Up to an input: no step is rendered and that input is the
                // answer.
                (0, inputs[index].name.clone())
            } else if let Some(index) = steps.iter().position(|s| s.name() == name) {
                (index + 1, steps[index].name().to_owned())
            } else {
                return Err(DefinitionError::at("upTo".to_owned(), UP_TO_UNKNOWN));
            }
        }
    };

    for (index, step) in steps[..limit].iter().enumerate() {
        match step {
            PipelineStep::Transform {
                reads, transforms, ..
            } => {
                for (position, text) in transforms.iter().enumerate() {
                    parse_transform(text).map_err(|_| {
                        DefinitionError::at(
                            format!("steps[{index}].transforms[{position}]"),
                            TRANSFORM_REFUSED,
                        )
                    })?;
                }
                // R1-B6 / D6a: a step names what it reads EXACTLY, case
                // included — `ClickHouse` matches a `WITH` name by exact case,
                // so a reference that differs only in case would be looked up
                // as a real table, unseen by the gate and the rewrite.
                let readable = inputs.iter().any(|i| i.name == *reads)
                    || steps[..index].iter().any(|s| s.name() == reads);
                if !readable {
                    let later = steps[..].iter().skip(index + 1).any(|s| s.name() == reads);
                    return Err(DefinitionError::step(
                        index,
                        if later {
                            READS_LATER_STEP
                        } else {
                            READS_NOT_INPUT
                        },
                    ));
                }
            }
            PipelineStep::Sql { sql, .. } => {
                // D5: one read-only statement of bounded length.
                check_pipeline_step_sql(sql)
                    .map_err(|refusal| DefinitionError::step(index, refusal.message()))?;
                // D6: every table it names is an input or an earlier step.
                check_step_tables(sql, index, inputs, steps)?;
            }
        }
    }
    Ok((limit, last))
}

/// D6: the tables one SQL step names, each resolved against the inputs and
/// the steps before it.
///
/// A fully qualified name must be an input's `<zone>.<table>`; a bare name
/// must be an input or an earlier step. Anything else is refused, naming this
/// step — `sql_rewrite::referenced_tables` leaves CTE names out of the
/// compiled text, so a bare name that names a LATER step would otherwise
/// pass unnoticed and fail in `ClickHouse` at run time.
fn check_step_tables(
    sql: &str,
    index: usize,
    inputs: &[PipelineInput],
    steps: &[PipelineStep],
) -> Result<(), DefinitionError> {
    let Some(tables) =
        crate::sql_rewrite::referenced_tables(sql, &sqlparser::dialect::ClickHouseDialect {})
    else {
        // `check_pipeline_step_sql` already proved it parses; this is the
        // defence-in-depth branch, refused rather than assumed.
        return Err(DefinitionError::step(index, READS_NOT_INPUT));
    };
    for table in tables {
        // R1-B6 / D6a: a step reads an input by its NAME in the pipeline, so
        // a qualified `zone.table` is refused even when it names an input —
        // `ClickHouse` would resolve it as a table in the connection's own
        // database, which neither the gate nor the rewrite sees.
        if table.contains('.') {
            return Err(DefinitionError::step(index, READS_NOT_INPUT));
        }
        // Exact, case included: the same reason as `reads` above.
        if inputs.iter().any(|i| i.name == table)
            || steps[..index].iter().any(|s| s.name() == table)
        {
            continue;
        }
        let later = steps[index + 1..].iter().any(|s| s.name() == table);
        return Err(DefinitionError::step(
            index,
            if later {
                READS_LATER_STEP
            } else {
                READS_NOT_INPUT
            },
        ));
    }
    Ok(())
}

/// The `FROM` expression for one input: `bronze` is read through the catalog
/// database as `` <catalog>.`bronze.<table>` ``, every other zone as
/// `` <zone>.`<table>` `` — the rule the orchestrator's `_resolve_source` used
/// to own (D4).
fn input_source(
    input: &PipelineInput,
    index: usize,
    catalog_db: Option<&str>,
) -> Result<String, DefinitionError> {
    if input.zone == "bronze" {
        let Some(db) = catalog_db else {
            return Err(DefinitionError::input(index, BRONZE_NEEDS_CATALOG));
        };
        return Ok(format!("{db}.`bronze.{}`", input.table));
    }
    Ok(format!("{}.`{}`", input.zone, input.table))
}

/// One step's CTE body, without the name (the caller adds it): D4's port of
/// the orchestrator's `_build_select_sql`.
///
/// The transform strings were already parsed by [`validate`]; this re-parses
/// rather than keeping a second value around, and reports the same refusal
/// with the same path if the two ever disagree.
fn render_step(step: &PipelineStep, index: usize) -> Result<String, DefinitionError> {
    match step {
        PipelineStep::Transform {
            name,
            reads,
            transforms,
        } => {
            let parsed = transforms
                .iter()
                .map(|text| {
                    parse_transform(text).map_err(|_| {
                        DefinitionError::at(format!("steps[{index}].transforms"), TRANSFORM_REFUSED)
                    })
                })
                .collect::<Result<Vec<Transform>, DefinitionError>>()?;
            Ok(format!(
                "  `{name}` AS ({})",
                render_transform_step(reads, &parsed)
            ))
        }
        PipelineStep::Sql { name, sql } => Ok(format!("  `{name}` AS (\n{sql}\n)")),
    }
}

/// Render one `transform` step as `SELECT ... FROM <reads> [WHERE ...]
/// [ORDER BY k LIMIT 1 BY k]`.
///
/// Same rules the orchestrator implemented in Python (D4): the last `select`
/// wins, `rename` and `cast` apply to the selected columns (`CAST` inside the
/// rename), filters are joined by `AND`, and the last `dedupe` key gives
/// `ORDER BY k LIMIT 1 BY k`. Every dynamic piece is rendered through
/// [`Transform::render_clickhouse`], never string-interpolated here.
#[must_use]
pub fn render_transform_step(reads: &str, transforms: &[Transform]) -> String {
    let mut columns: Option<Vec<String>> = None;
    let mut renames: Vec<(String, String)> = Vec::new();
    let mut casts: Vec<(String, String)> = Vec::new();
    let mut filters: Vec<String> = Vec::new();
    let mut dedupe_key: Option<String> = None;

    for transform in transforms {
        match transform {
            Transform::Select { columns: selected } => {
                columns = Some(selected.iter().map(ToString::to_string).collect());
            }
            Transform::Rename { from, to } => renames.push((from.to_string(), to.to_string())),
            Transform::Cast {
                column,
                target_type,
            } => casts.push((column.to_string(), (*target_type).to_owned())),
            Transform::Filter { .. } => filters.push(transform.render_clickhouse()),
            Transform::Dedupe { key } => dedupe_key = Some(key.to_string()),
        }
    }

    let select_columns = columns.map_or_else(
        || "*".to_owned(),
        |columns| {
            columns
                .iter()
                .map(|column| {
                    let mut expression = casts.iter().find(|(name, _)| name == column).map_or_else(
                        || column.clone(),
                        |(_, ty)| format!("CAST({column} AS {ty})"),
                    );
                    if let Some((_, to)) = renames.iter().find(|(from, _)| from == column) {
                        expression = format!("{expression} AS {to}");
                    }
                    expression
                })
                .collect::<Vec<_>>()
                .join(", ")
        },
    );

    let mut sql = format!("SELECT {select_columns} FROM `{reads}`");
    if !filters.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&filters.join(" AND "));
    }
    if let Some(key) = dedupe_key {
        use std::fmt::Write as _;
        let _ = write!(sql, " ORDER BY {key} LIMIT 1 BY {key}");
    }
    sql
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{DefinitionError, compile, validate_and_compile};

    fn input(name: &str, zone: &str, table: &str) -> lakehouse_store::pipelines::PipelineInput {
        lakehouse_store::pipelines::PipelineInput {
            name: name.to_owned(),
            zone: zone.to_owned(),
            table: table.to_owned(),
        }
    }

    fn transform_step(
        name: &str,
        reads: &str,
        transforms: &[&str],
    ) -> lakehouse_store::pipelines::PipelineStep {
        lakehouse_store::pipelines::PipelineStep::Transform {
            name: name.to_owned(),
            reads: reads.to_owned(),
            transforms: transforms.iter().map(|t| (*t).to_owned()).collect(),
        }
    }

    fn sql_step(name: &str, sql: &str) -> lakehouse_store::pipelines::PipelineStep {
        lakehouse_store::pipelines::PipelineStep::Sql {
            name: name.to_owned(),
            sql: sql.to_owned(),
        }
    }

    fn one_input() -> Vec<lakehouse_store::pipelines::PipelineInput> {
        vec![input("orders", "silver", "orders")]
    }

    fn refusal(
        inputs: &[lakehouse_store::pipelines::PipelineInput],
        steps: &[lakehouse_store::pipelines::PipelineStep],
    ) -> DefinitionError {
        compile(inputs, steps, None, Some("icecat_api")).expect_err("the definition is refused")
    }

    #[test]
    fn one_input_and_no_steps_is_a_plain_copy() {
        assert_eq!(
            compile(&one_input(), &[], None, Some("icecat_api")).unwrap(),
            "WITH\n  `orders` AS (SELECT * FROM silver.`orders`)\nSELECT * FROM `orders`"
        );
    }

    #[test]
    fn a_bronze_input_is_read_through_the_catalog_database() {
        let inputs = vec![input("g3a_orders", "bronze", "g3a_orders")];
        assert_eq!(
            compile(&inputs, &[], None, Some("icecat_api")).unwrap(),
            "WITH\n  `g3a_orders` AS (SELECT * FROM icecat_api.`bronze.g3a_orders`)\n\
             SELECT * FROM `g3a_orders`"
        );
    }

    #[test]
    fn a_bronze_input_without_a_catalog_database_is_refused() {
        let inputs = vec![input("g3a_orders", "bronze", "g3a_orders")];
        let err = compile(&inputs, &[], None, None).unwrap_err();
        assert_eq!(err.path(), "inputs[0]");
    }

    /// The statement `test_authored_factory.py:209` pinned (`SELECT id,
    /// CAST(amount AS Float64) AS amount_usd FROM silver.`orders` WHERE
    /// status = 'active' ORDER BY id LIMIT 1 BY id`) — now inside D4's
    /// `WITH`, rendered by this port instead of by Python.
    #[test]
    fn a_transform_step_renders_select_rename_cast_filter_and_dedupe() {
        let steps = vec![transform_step(
            "transformed",
            "orders",
            &[
                "select(id,amount)",
                "rename(amount,amount_usd)",
                "cast(amount,Float64)",
                "filter(status = 'active')",
                "dedupe(id)",
            ],
        )];
        assert_eq!(
            compile(&one_input(), &steps, None, Some("icecat_api")).unwrap(),
            "WITH\n  `orders` AS (SELECT * FROM silver.`orders`),\n  `transformed` AS \
             (SELECT id, CAST(amount AS Float64) AS amount_usd FROM `orders` WHERE status = \
             'active' ORDER BY id LIMIT 1 BY id)\nSELECT * FROM `transformed`"
        );
    }

    #[test]
    fn two_inputs_and_a_sql_step_render_one_statement() {
        let inputs = vec![
            input("orders", "bronze", "orders"),
            input("customers", "bronze", "customers"),
        ];
        let sql = "SELECT c.country, sum(o.amount) AS revenue FROM orders AS o \
                   LEFT JOIN customers AS c ON o.customer_id = c.id GROUP BY c.country";
        let steps = vec![sql_step("by_country", sql)];
        assert_eq!(
            compile(&inputs, &steps, None, Some("icecat_api")).unwrap(),
            format!(
                "WITH\n  `orders` AS (SELECT * FROM icecat_api.`bronze.orders`),\n  \
                 `customers` AS (SELECT * FROM icecat_api.`bronze.customers`),\n  \
                 `by_country` AS (\n{sql}\n)\nSELECT * FROM `by_country`"
            )
        );
    }

    /// D4: the SQL goes in between line breaks, so a comment at its end
    /// cannot swallow the closing bracket.
    #[test]
    fn a_sql_step_ending_in_a_line_comment_keeps_its_closing_bracket() {
        let steps = vec![sql_step("selected", "SELECT id FROM orders -- keep this")];
        let compiled = compile(&one_input(), &steps, None, Some("icecat_api")).unwrap();
        assert!(
            compiled.ends_with("\n)\nSELECT * FROM `selected`"),
            "{compiled}"
        );
    }

    #[test]
    fn up_to_an_input_compiles_that_input() {
        let inputs = vec![
            input("orders", "silver", "orders"),
            input("c", "silver", "c"),
        ];
        let steps = vec![sql_step("joined", "SELECT * FROM orders")];
        let compiled = compile(&inputs, &steps, Some("c"), Some("icecat_api")).unwrap();
        assert!(compiled.ends_with("SELECT * FROM `c`"), "{compiled}");
        assert!(!compiled.contains("`joined`"), "{compiled}");
    }

    #[test]
    fn up_to_a_middle_step_stops_there() {
        let inputs = vec![input("orders", "silver", "orders")];
        let steps = vec![
            sql_step("first", "SELECT id FROM orders"),
            sql_step("second", "SELECT id FROM first"),
            sql_step("third", "SELECT id FROM second"),
        ];
        let compiled = compile(&inputs, &steps, Some("second"), Some("icecat_api")).unwrap();
        assert!(compiled.ends_with("SELECT * FROM `second`"), "{compiled}");
        assert!(!compiled.contains("`third`"), "{compiled}");
    }

    #[test]
    fn up_to_an_unknown_name_is_refused() {
        let inputs = one_input();
        let steps = vec![sql_step("first", "SELECT id FROM orders")];
        let err = compile(&inputs, &steps, Some("nope"), Some("icecat_api")).unwrap_err();
        assert_eq!(err.path(), "upTo");
    }

    #[test]
    fn a_sixth_input_is_refused_at_its_own_index() {
        let inputs = (0..6)
            .map(|i| input(&format!("i{i}"), "silver", &format!("t{i}")))
            .collect::<Vec<_>>();
        let err = refusal(&inputs, &[]);
        assert_eq!(err.path(), "inputs[5]");
    }

    #[test]
    fn a_twenty_first_step_is_refused_at_its_own_index() {
        let steps = (0..21)
            .map(|i| sql_step(&format!("s{i}"), "SELECT id FROM orders"))
            .collect::<Vec<_>>();
        let err = refusal(&one_input(), &steps);
        assert_eq!(err.path(), "steps[20]");
    }

    #[test]
    fn a_duplicate_name_differing_only_in_case_is_refused() {
        let steps = vec![sql_step("Orders", "SELECT id FROM orders")];
        let err = refusal(&one_input(), &steps);
        assert_eq!(err.path(), "steps[0]");
        assert_eq!(err.message(), super::DUPLICATE_NAME);
    }

    #[test]
    fn a_name_starting_with_underscores_is_refused() {
        let steps = vec![sql_step("__internal", "SELECT id FROM orders")];
        let err = refusal(&one_input(), &steps);
        assert_eq!(err.path(), "steps[0]");
        assert_eq!(err.message(), super::INVALID_NAME);
    }

    #[test]
    fn a_read_of_a_later_step_is_refused() {
        let steps = vec![
            sql_step("first", "SELECT * FROM second"),
            sql_step("second", "SELECT id FROM orders"),
        ];
        let err = refusal(&one_input(), &steps);
        assert_eq!(err.path(), "steps[0]");
        assert_eq!(err.message(), super::READS_LATER_STEP);
    }

    #[test]
    fn a_transform_step_reading_a_later_step_is_refused() {
        let steps = vec![
            transform_step("first", "second", &["select(id)"]),
            sql_step("second", "SELECT id FROM orders"),
        ];
        let err = refusal(&one_input(), &steps);
        assert_eq!(err.path(), "steps[0]");
        assert_eq!(err.message(), super::READS_LATER_STEP);
    }

    #[test]
    fn a_sql_step_that_is_not_a_select_is_refused_at_its_index() {
        for sql in [
            "DROP TABLE silver.orders",
            "INSERT INTO silver.orders VALUES (1)",
        ] {
            let steps = vec![sql_step("bad", sql)];
            let err = refusal(&one_input(), &steps);
            assert_eq!(err.path(), "steps[0]", "{sql}");
        }
    }

    #[test]
    fn a_sql_step_with_two_statements_is_refused() {
        let steps = vec![sql_step(
            "two",
            "SELECT id FROM orders; SELECT id FROM orders",
        )];
        let err = refusal(&one_input(), &steps);
        assert_eq!(err.path(), "steps[0]");
    }

    #[test]
    fn a_sql_step_with_settings_is_refused() {
        let steps = vec![sql_step(
            "settings",
            "SELECT id FROM orders SETTINGS max_threads = 1",
        )];
        let err = refusal(&one_input(), &steps);
        assert_eq!(err.path(), "steps[0]");
    }

    #[test]
    fn a_sql_step_reading_a_table_outside_the_inputs_is_refused() {
        let steps = vec![sql_step("other", "SELECT id FROM silver.other")];
        let err = refusal(&one_input(), &steps);
        assert_eq!(err.path(), "steps[0]");
        assert_eq!(err.message(), super::READS_NOT_INPUT);
    }

    #[test]
    fn a_sql_step_reading_a_bare_unknown_name_is_refused() {
        let steps = vec![sql_step("unknown", "SELECT id FROM nowhere")];
        let err = refusal(&one_input(), &steps);
        assert_eq!(err.path(), "steps[0]");
        assert_eq!(err.message(), super::READS_NOT_INPUT);
    }

    #[test]
    fn an_unsafe_zone_or_table_is_refused_at_its_input() {
        let inputs = vec![input("orders", "silver", "orders; DROP TABLE x")];
        let err = refusal(&inputs, &[]);
        assert_eq!(err.path(), "inputs[0]");
        assert_eq!(err.message(), super::INVALID_TABLE);
    }

    #[test]
    fn an_output_equal_to_an_input_is_refused_at_that_input() {
        let inputs = vec![input("orders", "silver", "orders")];
        let err =
            validate_and_compile(&inputs, &[], Some(("silver", "orders")), Some("icecat_api"))
                .unwrap_err();
        assert_eq!(err.path(), "inputs[0]");
        assert_eq!(err.message(), super::OUTPUT_IS_INPUT);
    }

    #[test]
    fn a_transform_string_outside_the_grammar_names_its_position() {
        let steps = vec![transform_step(
            "bad",
            "orders",
            &["select(id)", "filter(1=1; DROP TABLE x)"],
        )];
        let err = refusal(&one_input(), &steps);
        assert_eq!(err.path(), "steps[0].transforms[1]");
    }

    #[test]
    fn a_read_that_differs_only_in_case_is_refused() {
        // R1-B6 / D6a: an input named `SECRET`; `FROM secret` must not be
        // taken as that input — ClickHouse would look up `secret` as a table.
        let inputs = vec![input("SECRET", "silver", "secret")];
        let steps = vec![sql_step("copied", "SELECT * FROM secret")];
        let err = refusal(&inputs, &steps);
        assert_eq!(err.path(), "steps[0]");
        assert_eq!(err.message(), super::READS_NOT_INPUT);
    }

    #[test]
    fn a_transform_step_read_that_differs_only_in_case_is_refused() {
        let inputs = vec![input("Orders", "silver", "orders")];
        let steps = vec![transform_step("copied", "orders", &["select(id)"])];
        let err = refusal(&inputs, &steps);
        assert_eq!(err.path(), "steps[0]");
        assert_eq!(err.message(), super::READS_NOT_INPUT);
    }

    #[test]
    fn a_qualified_name_in_a_sql_step_is_refused_even_when_it_is_an_input() {
        // D6a: a step reads an input by its name in the pipeline.
        let inputs = vec![input("orders", "silver", "orders")];
        let steps = vec![sql_step("copied", "SELECT * FROM silver.orders")];
        let err = refusal(&inputs, &steps);
        assert_eq!(err.path(), "steps[0]");
        assert_eq!(err.message(), super::READS_NOT_INPUT);
    }

    #[test]
    fn up_to_an_input_named_in_another_case_is_refused() {
        let inputs = vec![input("orders", "silver", "orders")];
        let err = compile(&inputs, &[], Some("Orders"), Some("icecat_api")).unwrap_err();
        assert_eq!(err.path(), "upTo");
    }

    /// Decision 12: window functions are allowed, and so is every other
    /// shape of one `SELECT` a person writes by hand.
    #[test]
    fn window_functions_subqueries_own_with_and_limit_by_are_accepted() {
        let steps = vec![
            sql_step(
                "ranked",
                "SELECT id, row_number() OVER (PARTITION BY customer ORDER BY amount DESC) AS rn \
                 FROM orders",
            ),
            sql_step(
                "nested",
                "WITH picked AS (SELECT id FROM ranked WHERE rn = 1) \
                 SELECT * FROM (SELECT id FROM picked) AS inner_q",
            ),
            sql_step("one_per", "SELECT id FROM nested ORDER BY id LIMIT 1 BY id"),
        ];
        let compiled = compile(&one_input(), &steps, None, Some("icecat_api")).unwrap();
        assert!(compiled.contains("row_number() OVER"), "{compiled}");
    }
}
