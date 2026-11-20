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
//! 1. 1 to 5 inputs and at most 20 steps (D2, decisions 4 and 15). A
//!    definition with no step copies its one input (D1); several inputs and
//!    no step are refused, because a copy of the first would silently ignore
//!    the rest — unless `up_to` asks for one input alone, which is the
//!    editor's preview of a pipeline whose step is not written yet
//!    (BLD-1 part 1 D1 / S2 review R1-S1).
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
//! 7. BLD-16 (`BLD-1` part 2) / J1: a `join`, `aggregate` or `filter` step
//!    reads an input or an earlier step by the same exact rule (D6a), and
//!    its fields parse into the checked forms of
//!    [`crate::pipeline_forms`] — the module that renders each into the one
//!    `SELECT` that is the step's CTE body, so the body the compiler puts
//!    in the `WITH` and the text [`step_sql`] hands the console are
//!    produced by the same functions and cannot differ.

use lakehouse_core::ident::Ident;
use lakehouse_store::pipelines::{PipelineInput, PipelineStep};

use crate::pipeline_forms;
use crate::sql_guard::check_pipeline_step_sql;
use crate::transform_grammar::{Transform, parse_transform};

/// At most 5 inputs per pipeline (decision 4).
pub const MAX_INPUTS: usize = 5;
/// At most 20 steps per pipeline (decision 15).
pub const MAX_STEPS: usize = 20;
/// Longest input or step name, in characters (D2).
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
    /// BLD-16: `pub(crate)` so `pipeline_forms` refuses with the same two
    /// fields (path + fixed text) instead of growing a second constructor
    /// the routes would have to understand as well.
    pub(crate) fn at(path: String, message: &'static str) -> Self {
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
/// BLD-1 part 1 D1 / S2 review R1-S1: with no step there is exactly one
/// input, which the pipeline copies. Several inputs and no step would compile
/// to a copy of the first and silently ignore the others.
const NO_STEP_WITH_MANY_INPUTS: &str = "with more than one input, add a step that reads them";
/// BLD-16 / J2: `pub(crate)` so `pipeline_forms` reuses D2's own sentence for
/// a measure's name rather than writing a near-copy of it.
pub(crate) const INVALID_NAME: &str = "a name must be letters, digits and underscores (not starting with a digit or __), \
     at most 48 characters";
const DUPLICATE_NAME: &str = "this name is already used by another input or step";
const INVALID_TABLE: &str =
    "a zone and a table must be plain names (letters, digits and underscores)";
const READS_NOT_INPUT: &str = "this step reads a table that is not an input; add it under Inputs";
const READS_LATER_STEP: &str = "this step reads a step that comes after it; reorder the steps";
const TRANSFORM_REFUSED: &str = "this transform is not in the supported grammar";
/// BLD-11 review R5: a rename that would leave two output columns with one
/// name is refused when the pipeline is saved, not left to fail on its next
/// run with a duplicate-column error from the engine.
const DUPLICATE_OUTPUT_COLUMN: &str = "two columns would have the same name after this step; give the renamed column a different name";
const OUTPUT_IS_INPUT: &str =
    "the output table is one of the inputs; the output cannot be one of the inputs";
const BRONZE_NEEDS_CATALOG: &str =
    "a bronze input is read through the catalog database, which this deployment has not configured";
const UP_TO_UNKNOWN: &str = "upTo must name an input or a step of this pipeline";
/// BLD-16 / J6: [`step_sql`] was asked for a name that is not a step of
/// this pipeline — an input's name included, because the answer is the body
/// of a step's `WITH` entry and an input has none.
const STEP_UNKNOWN: &str = "step must name a step of this pipeline";

/// Whether `name` is one D2 allows for an input or a step.
///
/// BLD-16 / J2: `pub(crate)` so `pipeline_forms` applies the SAME rule to a
/// measure's name (a group-by column and a measure become output columns, so
/// their names live in the same namespace as a step's).
pub(crate) fn valid_name(name: &str) -> bool {
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
    // BLD-1 part 1 review R1-B1 / D4a: a definition of today's shape compiles
    // to today's statement, with no `WITH`. That is what `DATA-13`'s lineage
    // reads, and a `WITH` made every existing pipeline's mapping partial.
    if is_legacy_shape(inputs, steps) {
        let source = input_source(&inputs[0], 0, catalog_db)?;
        if limit == 0 {
            return Ok(format!("SELECT * FROM {source}"));
        }
        let PipelineStep::Transform { transforms, .. } = &steps[0] else {
            // `is_legacy_shape` accepted only that kind.
            return Ok(format!("SELECT * FROM {source}"));
        };
        let parsed = parse_transforms(transforms, 0)?;
        return Ok(render_transform_step(&source, &parsed));
    }
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

/// Parse a `transform` step's strings, naming the position at fault.
fn parse_transforms(
    transforms: &[String],
    index: usize,
) -> Result<Vec<Transform>, DefinitionError> {
    transforms
        .iter()
        .map(|text| {
            parse_transform(text).map_err(|_| {
                DefinitionError::at(format!("steps[{index}].transforms"), TRANSFORM_REFUSED)
            })
        })
        .collect()
}

/// BLD-11 review R5: refuse a transform step whose own transforms would emit
/// two columns with one name. With an explicit `select` the emitted names are
/// the selected columns after renaming; without one, the renamed columns are
/// appended to `*` and only collisions between rename targets are knowable —
/// the source table's columns are not part of a saved definition.
fn check_output_names(parsed: &[Transform], index: usize) -> Result<(), DefinitionError> {
    let renames: Vec<(&Ident, &Ident)> = parsed
        .iter()
        .filter_map(|transform| match transform {
            Transform::Rename { from, to } => Some((from, to)),
            _ => None,
        })
        .collect();
    let emitted: Vec<String> = match parsed.iter().find_map(|transform| match transform {
        Transform::Select { columns } => Some(columns),
        _ => None,
    }) {
        Some(selected) => selected
            .iter()
            .map(|column| {
                renames
                    .iter()
                    .find(|(from, _)| *from == column)
                    .map_or_else(|| column.to_string(), |(_, to)| to.to_string())
            })
            .collect(),
        None => renames.iter().map(|(_, to)| to.to_string()).collect(),
    };
    let mut seen = std::collections::HashSet::new();
    for name in emitted {
        if !seen.insert(name) {
            return Err(DefinitionError::step(index, DUPLICATE_OUTPUT_COLUMN));
        }
    }
    Ok(())
}

/// D4a: whether this definition is today's shape — one input and either no
/// step or exactly one `transform` step reading it — which is what compiles
/// to today's statement (no `WITH`), and the only shape whose statement is
/// noted for column lineage (D4b).
#[must_use]
pub fn is_legacy_shape(inputs: &[PipelineInput], steps: &[PipelineStep]) -> bool {
    let [input] = inputs else {
        return false;
    };
    match steps {
        [] => true,
        [PipelineStep::Transform { reads, .. }] => reads == &input.name,
        _ => false,
    }
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
    // BLD-1 part 1 D1 / S2 review R1-S1: "with no steps there is exactly one
    // input and the pipeline copies it." Several inputs and no step would
    // compile to `SELECT * FROM <the first>` and ignore the others, so the
    // whole definition is refused at the second input. `up_to` is exempt: it
    // asks for ONE input (the editor previews an input before its step is
    // written), not for the definition.
    if up_to.is_none() && steps.is_empty() && inputs.len() > 1 {
        return Err(DefinitionError::input(1, NO_STEP_WITH_MANY_INPUTS));
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
        // BLD-16 / J1 moved the per-step rules into `check_step` so the save,
        // the preview, the run and the compile route all refuse identically.
        check_step(step, index, inputs, steps)?;
    }
    Ok((limit, last))
}

/// One step's own rules (the per-step `match` of [`validate`]): the read
/// check every kind makes (D6a) and each kind's own parsing — `transform`
/// strings, a SQL step's one read-only statement, or a form kind's fields
/// through [`pipeline_forms`], so the save, the preview, the run and the
/// compile route all refuse identically (D9).
///
/// # Errors
///
/// [`DefinitionError`] naming the step and, for a form kind, the field at
/// fault.
fn check_step(
    step: &PipelineStep,
    index: usize,
    inputs: &[PipelineInput],
    steps: &[PipelineStep],
) -> Result<(), DefinitionError> {
    match step {
        PipelineStep::Transform {
            reads, transforms, ..
        } => {
            let parsed = transforms
                .iter()
                .enumerate()
                .map(|(position, text)| {
                    parse_transform(text).map_err(|_| {
                        DefinitionError::at(
                            format!("steps[{index}].transforms[{position}]"),
                            TRANSFORM_REFUSED,
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            // BLD-11 review R5: a duplicate output name is refused here, at
            // save, instead of failing on the pipeline's next run. Re-applied
            // on top of BLD-16's `check_step` extraction in the merge.
            check_output_names(&parsed, index)?;
            // R1-B6 / D6a: a step names what it reads EXACTLY, case
            // included — `ClickHouse` matches a `WITH` name by exact case,
            // so a reference that differs only in case would be looked up
            // as a real table, unseen by the gate and the rewrite.
            // `None` keeps this step's existing `steps[i]` path, whose
            // message no change may alter.
            check_read(index, None, reads, inputs, steps)?;
        }
        PipelineStep::Sql { sql, .. } => {
            // D5: one read-only statement of bounded length.
            check_pipeline_step_sql(sql)
                .map_err(|refusal| DefinitionError::step(index, refusal.message()))?;
            // D6: every table it names is an input or an earlier step.
            check_step_tables(sql, index, inputs, steps)?;
        }
        // BLD-16 / J1: the three form kinds. The read check is the
        // compiler's (D6a — a reference must name an input or an
        // earlier step, exactly); everything else is
        // [`pipeline_forms`]' own parsing, so the save, the preview, the
        // run and the compile route all refuse identically (D9).
        PipelineStep::Join {
            left,
            right,
            join_type,
            on,
            columns,
            ..
        } => {
            check_read(index, Some("left"), left, inputs, steps)?;
            check_read(index, Some("right"), right, inputs, steps)?;
            pipeline_forms::parse_join(index, join_type, on, columns)?;
        }
        PipelineStep::Aggregate {
            reads,
            group_by,
            measures,
            having_match,
            having,
            ..
        } => {
            check_read(index, Some("reads"), reads, inputs, steps)?;
            pipeline_forms::parse_aggregate(index, group_by, measures, having_match, having)?;
        }
        PipelineStep::Filter {
            reads,
            match_mode,
            conditions,
            ..
        } => {
            check_read(index, Some("reads"), reads, inputs, steps)?;
            pipeline_forms::parse_filter(index, match_mode, conditions)?;
        }
    }
    Ok(())
}

/// D6a: a step names what it reads EXACTLY, case included — `ClickHouse`
/// matches a `WITH` name by exact case, so a reference that differs only in
/// case would be looked up as a real table, unseen by the gate and the
/// rewrite.
///
/// `field` says which stored field the reference came from, so the refusal
/// names it (BLD-16 / J2: `steps[i].left`, `.right`, `.reads`); `None` keeps
/// the `transform` step's existing `steps[i]` path, whose message no change
/// may alter.
///
/// # Errors
///
/// [`DefinitionError`] naming the field at fault: `READS_LATER_STEP` when
/// the name belongs to a step after this one, `READS_NOT_INPUT` otherwise.
fn check_read(
    index: usize,
    field: Option<&str>,
    reads: &str,
    inputs: &[PipelineInput],
    steps: &[PipelineStep],
) -> Result<(), DefinitionError> {
    let readable =
        inputs.iter().any(|i| i.name == reads) || steps[..index].iter().any(|s| s.name() == reads);
    if readable {
        return Ok(());
    }
    let later = steps[..].iter().skip(index + 1).any(|s| s.name() == reads);
    let message = if later {
        READS_LATER_STEP
    } else {
        READS_NOT_INPUT
    };
    match field {
        Some(field) => Err(DefinitionError::at(
            format!("steps[{index}].{field}"),
            message,
        )),
        None => Err(DefinitionError::step(index, message)),
    }
}

/// The names one step declares as `WITH` aliases, and the tables it reads —
/// both AS WRITTEN, at any depth.
///
/// BLD-1 part 1 review R2-B1 / D6a: this walks the parsed step with
/// `sqlparser`'s visitor instead of `sql_rewrite::referenced_tables`, because
/// that collector lowercases every name and drops a bare name that equals one
/// of the statement's own `WITH` names — so `FROM ORDERS` was accepted as the
/// input `orders`, `WITH T AS … SELECT * FROM t` was never checked, and an
/// input `Orders` could never be read at all. `ClickHouse` matches a `WITH`
/// name by exact case, so these comparisons must keep it.
///
/// A table function (`args.is_some()`) is skipped: the rewrite classifies and
/// refuses it, as it does for every other surface.
#[derive(Default)]
struct StepReads {
    /// Every `WITH` name the step declares, anywhere in it.
    with_names: Vec<String>,
    /// Every ordinary table relation, at any depth, as written.
    tables: Vec<sqlparser::ast::ObjectName>,
}

impl sqlparser::ast::Visitor for StepReads {
    type Break = std::convert::Infallible;

    fn pre_visit_query(
        &mut self,
        query: &sqlparser::ast::Query,
    ) -> std::ops::ControlFlow<Self::Break> {
        if let Some(with) = &query.with {
            for cte in &with.cte_tables {
                self.with_names.push(cte.alias.name.value.clone());
            }
        }
        std::ops::ControlFlow::Continue(())
    }

    fn pre_visit_table_factor(
        &mut self,
        factor: &sqlparser::ast::TableFactor,
    ) -> std::ops::ControlFlow<Self::Break> {
        if let sqlparser::ast::TableFactor::Table { name, args, .. } = factor
            && args.is_none()
        {
            self.tables.push(name.clone());
        }
        std::ops::ControlFlow::Continue(())
    }
}

/// Every identifier part of a name, as written. A part that is a function
/// (`FUNCTION(...)`) is not an identifier, so the name is refused rather than
/// guessed at.
fn name_parts(name: &sqlparser::ast::ObjectName) -> Option<Vec<String>> {
    name.0
        .iter()
        .map(|part| part.as_ident().map(|ident| ident.value.clone()))
        .collect()
}

/// D6: the names one SQL step reads, each resolved against the inputs and the
/// steps before it.
///
/// D6a, and review R2-B1: a reference matches by its name EXACTLY, case
/// included, against an input, an earlier step, or a `WITH` name the step
/// itself declares (also as written). A name with more than one part
/// (`zone.table`) is refused even when it names an input, and a bare name that
/// names a LATER step is refused with its own sentence. Anything else is not a
/// table the pipeline reads.
fn check_step_tables(
    sql: &str,
    index: usize,
    inputs: &[PipelineInput],
    steps: &[PipelineStep],
) -> Result<(), DefinitionError> {
    use sqlparser::ast::Statement::Query as QueryStatement;

    let dialect = sqlparser::dialect::ClickHouseDialect {};
    let statements = sqlparser::parser::Parser::parse_sql(&dialect, sql)
        .map_err(|_| DefinitionError::step(index, READS_NOT_INPUT))?;
    // `check_pipeline_step_sql` already proved one query; this is the
    // defence-in-depth branch, refused rather than assumed.
    let [QueryStatement(query)] = statements.as_slice() else {
        return Err(DefinitionError::step(index, READS_NOT_INPUT));
    };
    let mut reads = StepReads::default();
    let _: std::ops::ControlFlow<_> = sqlparser::ast::Visit::visit(query, &mut reads);

    for name in &reads.tables {
        let Some(parts) = name_parts(name) else {
            return Err(DefinitionError::step(index, READS_NOT_INPUT));
        };
        let [table] = parts.as_slice() else {
            // D6a: a step reads an input by its name in the pipeline.
            return Err(DefinitionError::step(index, READS_NOT_INPUT));
        };
        if inputs.iter().any(|input| &input.name == table)
            || steps[..index].iter().any(|step| step.name() == table)
            || reads.with_names.iter().any(|with| with == table)
        {
            continue;
        }
        let later = steps[index + 1..].iter().any(|step| step.name() == table);
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

/// One step's CTE entry, without the name (the caller adds it): D4's port of
/// the orchestrator's `_build_select_sql`.
///
/// A SQL step keeps its own line breaks around the text (so a `--` comment
/// at its end cannot swallow the closing bracket); every other kind is one
/// line, the body from [`step_body`] — the same function [`step_sql`]
/// answers with, so the two cannot differ (BLD-16 / J6).
fn render_step(step: &PipelineStep, index: usize) -> Result<String, DefinitionError> {
    let name = step.name();
    match step {
        PipelineStep::Sql { sql, .. } => Ok(format!("  `{name}` AS (\n{sql}\n)")),
        _ => Ok(format!("  `{name}` AS ({})", step_body(step, index)?)),
    }
}

/// The `SELECT` of one step over the inputs and the earlier steps by their
/// NAMES (BLD-16 / J6): the body [`render_step`] puts inside a step's `WITH`
/// entry and [`step_sql`] answers with — before the access rewrite, because
/// the rewritten text would show another person's masking expressions, and
/// a SQL step is written over names, not tables.
///
/// # Errors
///
/// [`DefinitionError`] at the step's own index when its fields do not
/// parse; the same refusal [`validate`] already gave, so the two paths
/// cannot disagree.
fn step_body(step: &PipelineStep, index: usize) -> Result<String, DefinitionError> {
    match step {
        PipelineStep::Transform {
            reads, transforms, ..
        } => {
            let parsed = parse_transforms(transforms, index)?;
            Ok(render_transform_step(&format!("`{reads}`"), &parsed))
        }
        PipelineStep::Sql { sql, .. } => Ok(sql.clone()),
        PipelineStep::Join {
            left,
            right,
            join_type,
            on,
            columns,
            ..
        } => {
            let form = pipeline_forms::parse_join(index, join_type, on, columns)?;
            Ok(pipeline_forms::render_join(left, right, &form))
        }
        PipelineStep::Aggregate {
            reads,
            group_by,
            measures,
            having_match,
            having,
            ..
        } => {
            let form =
                pipeline_forms::parse_aggregate(index, group_by, measures, having_match, having)?;
            Ok(pipeline_forms::render_aggregate(reads, &form))
        }
        PipelineStep::Filter {
            reads,
            match_mode,
            conditions,
            ..
        } => {
            let form = pipeline_forms::parse_filter(index, match_mode, conditions)?;
            Ok(pipeline_forms::render_filter(reads, &form))
        }
    }
}

/// Build the projection for a transform step that has no explicit `select`
/// (BLD-11 F7, D11). A plain `*` would ignore `cast`/`rename` entirely, so
/// renamed columns are cut from `*` with `EXCEPT` and appended at the end as
/// `<old> AS <new>` (cast under the new name when it is also cast), while
/// casts on columns that keep their name go into `REPLACE`. `EXCEPT` comes
/// before `REPLACE` — the order `sqlparser`'s `ClickHouse` dialect parses. With
/// neither transform this is the unchanged `*`.
fn render_star_with_transforms(renames: &[(String, String)], casts: &[(String, String)]) -> String {
    use std::fmt::Write as _;

    if renames.is_empty() && casts.is_empty() {
        return "*".to_owned();
    }
    let mut projection = "*".to_owned();
    if !renames.is_empty() {
        let excluded = renames
            .iter()
            .map(|(from, _)| from.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let _ = write!(projection, " EXCEPT ({excluded})");
    }
    // A cast on a renamed column is applied to the appended expression, not
    // to `*`; only the columns that keep their name are replaced in place.
    let in_place = casts
        .iter()
        .filter(|(column, _)| !renames.iter().any(|(from, _)| from == column))
        .map(|(column, ty)| format!("CAST({column} AS {ty}) AS {column}"))
        .collect::<Vec<_>>();
    if !in_place.is_empty() {
        let _ = write!(projection, " REPLACE ({})", in_place.join(", "));
    }
    for (from, to) in renames {
        let expression = casts
            .iter()
            .find(|(column, _)| column == from)
            .map_or_else(|| from.clone(), |(_, ty)| format!("CAST({from} AS {ty})"));
        let _ = write!(projection, ", {expression} AS {to}");
    }
    projection
}

/// BLD-16 / J6: the SQL of one form step, as a person reads it — the body
/// the compiler puts inside that step's `WITH` entry, over the inputs and
/// the earlier steps by their names, before the access rewrite (the
/// rewritten text would show another person's masking expressions, and a
/// SQL step is written over names, not tables). Nothing is stored and no
/// table is read; the route that answers this sends nothing to
/// `ClickHouse`.
///
/// "Switch to SQL step" is the console's: it replaces the form step with
/// `{ name, kind: "sql", sql: <this answer> }`, same name, same place —
/// and because the answer IS the CTE body, the compiled statement the
/// pipeline runs does not change with it.
///
/// # Errors
///
/// [`DefinitionError`] at `step` when `step_name` is not a step's name
/// exactly (an input's name included, because an input has no `WITH`
/// entry); otherwise any refusal of [`validate`] for the definition up to
/// that step.
pub fn step_sql(
    inputs: &[PipelineInput],
    steps: &[PipelineStep],
    step_name: &str,
) -> Result<String, DefinitionError> {
    // The name check comes first so an input's name is refused as "not a
    // step", not answered as the preview of that input (`up_to`'s own
    // sentence would name the route's field, not `step`).
    let Some(index) = steps.iter().position(|step| step.name() == step_name) else {
        return Err(DefinitionError::at("step".to_owned(), STEP_UNKNOWN));
    };
    // D2 to D6 on the part up to the named step: a broken earlier step is
    // refused at its own index, and nothing beyond the named one is read.
    validate(inputs, steps, Some(step_name))?;
    step_body(&steps[index], index)
}

/// Render one `transform` step as `SELECT ... FROM <reads> [WHERE ...]
/// [ORDER BY k LIMIT 1 BY k]`.
///
/// Same rules the orchestrator implemented in Python (D4): the last `select`
/// wins, `rename` and `cast` apply to the selected columns (`CAST` inside the
/// rename), filters are joined by `AND`, and the last `dedupe` key gives
/// `ORDER BY k LIMIT 1 BY k`. Every dynamic piece is rendered through
/// [`Transform::render_clickhouse`], never string-interpolated here.
///
/// BLD-11 F7 (D11): with an explicit `select`, nothing below changes. With no
/// `select` the step used to be plain `SELECT *`, silently dropping every
/// `cast`/`rename` (the Gold table kept the source type). Now a cast or
/// rename with no select is applied to all columns through `ClickHouse`'s
/// `* EXCEPT`/`* REPLACE`: renamed columns are excluded from `*` and appended
/// at the end as `<old> AS <new>` (or `CAST(<old> AS T) AS <new>` when cast),
/// while cast columns that keep their name are replaced in place.
#[must_use]
pub fn render_transform_step(from: &str, transforms: &[Transform]) -> String {
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
        || render_star_with_transforms(&renames, &casts),
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

    let mut sql = format!("SELECT {select_columns} FROM {from}");
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

    /// Two inputs: the definition D1 says needs a step (`R1-S1`).
    fn two_inputs() -> Vec<lakehouse_store::pipelines::PipelineInput> {
        vec![
            input("orders", "silver", "orders"),
            input("customers", "silver", "customers"),
        ]
    }

    fn refusal(
        inputs: &[lakehouse_store::pipelines::PipelineInput],
        steps: &[lakehouse_store::pipelines::PipelineStep],
    ) -> DefinitionError {
        compile(inputs, steps, None, Some("icecat_api")).expect_err("the definition is refused")
    }

    /// D4a: today's shape compiles to today's statement — the text
    /// `_build_select_sql` gave on `main` for the same pipeline. `DATA-13`'s
    /// lineage reads it, and a `WITH` made every mapping partial (R1-B1).
    #[test]
    fn one_input_and_no_steps_is_a_plain_copy() {
        assert_eq!(
            compile(&one_input(), &[], None, Some("icecat_api")).unwrap(),
            "SELECT * FROM silver.`orders`"
        );
    }

    /// BLD-1 part 1 D1 / S2 review R1-S1: "with no steps there is exactly one
    /// input and the pipeline copies it." Two inputs and no step used to
    /// compile to a copy of the first, silently ignoring the second.
    #[test]
    fn two_inputs_and_no_step_is_refused_at_the_second_input() {
        let inputs = two_inputs();
        let err = refusal(&inputs, &[]);
        assert_eq!(err.path(), "inputs[1]");
        assert_eq!(
            err.message(),
            "with more than one input, add a step that reads them"
        );
    }

    /// R1-S1: the same definition still previews ONE input, which is what the
    /// editor asks before its step is written (`up_to`).
    #[test]
    fn two_inputs_and_no_step_still_previews_the_named_input() {
        assert_eq!(
            compile(&two_inputs(), &[], Some("customers"), Some("icecat_api")).unwrap(),
            "WITH\n  `orders` AS (SELECT * FROM silver.`orders`),\n  \
             `customers` AS (SELECT * FROM silver.`customers`)\nSELECT * FROM `customers`"
        );
    }

    /// R1-S1: the refusal is about a definition with NO step; one step over
    /// two inputs compiles as before.
    #[test]
    fn two_inputs_and_one_step_are_not_refused() {
        let steps = vec![transform_step(
            "paid",
            "orders",
            &["filter(status = 'paid')"],
        )];
        assert_eq!(
            compile(&two_inputs(), &steps, None, Some("icecat_api")).unwrap(),
            "WITH\n  `orders` AS (SELECT * FROM silver.`orders`),\n  \
             `customers` AS (SELECT * FROM silver.`customers`),\n  \
             `paid` AS (SELECT * FROM `orders` WHERE status = 'paid')\n\
             SELECT * FROM `paid`"
        );
    }

    #[test]
    fn a_bronze_input_is_read_through_the_catalog_database() {
        let inputs = vec![input("g3a_orders", "bronze", "g3a_orders")];
        assert_eq!(
            compile(&inputs, &[], None, Some("icecat_api")).unwrap(),
            "SELECT * FROM icecat_api.`bronze.g3a_orders`"
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
            "SELECT id, CAST(amount AS Float64) AS amount_usd FROM silver.`orders` \
             WHERE status = 'active' ORDER BY id LIMIT 1 BY id"
        );
    }

    fn render_from(from: &str, transforms: &[&str]) -> String {
        let parsed: Vec<crate::transform_grammar::Transform> = transforms
            .iter()
            .map(|text| crate::transform_grammar::parse_transform(text).expect("a valid transform"))
            .collect();
        super::render_transform_step(from, &parsed)
    }

    fn render(transforms: &[&str]) -> String {
        render_from("silver.`orders`", transforms)
    }

    /// BLD-11 F7 (D11): a cast with no `select` used to be dropped (plain
    /// `SELECT *`), leaving the Gold column at the source type. It now goes
    /// into `* REPLACE`, replacing the column in place.
    #[test]
    fn a_cast_with_no_select_replaces_the_column_in_place() {
        assert_eq!(
            render(&["cast(amount,Float64)"]),
            "SELECT * REPLACE (CAST(amount AS Float64) AS amount) FROM silver.`orders`"
        );
    }

    /// BLD-11 F7 (D11): a rename with no `select` used to be dropped. It now
    /// cuts the old name from `*` and appends the renamed column at the end.
    #[test]
    fn a_rename_with_no_select_moves_the_column_to_the_end() {
        assert_eq!(
            render(&["rename(amount,amount_usd)"]),
            "SELECT * EXCEPT (amount), amount AS amount_usd FROM silver.`orders`"
        );
    }

    /// BLD-11 F7 (D11): a column cast and renamed with no `select` is cast
    /// under its new name, appended at the end.
    #[test]
    fn a_cast_and_rename_of_one_column_with_no_select_cast_it_under_its_new_name() {
        assert_eq!(
            render(&["rename(amount,amount_usd)", "cast(amount,Float64)"]),
            "SELECT * EXCEPT (amount), CAST(amount AS Float64) AS amount_usd FROM silver.`orders`"
        );
    }

    /// BLD-11 F7: a filter or dedupe with no `select` still selects every
    /// column — this is today's behaviour and stays unchanged.
    #[test]
    fn a_filter_or_dedupe_alone_still_selects_star() {
        assert_eq!(
            render(&["filter(status = 'active')", "dedupe(id)"]),
            "SELECT * FROM silver.`orders` WHERE status = 'active' ORDER BY id LIMIT 1 BY id"
        );
    }

    /// BLD-11 F7 (D11): the rendered `* EXCEPT`/`* REPLACE` statements must
    /// be SQL the access rewrite can parse; otherwise a transform step would
    /// compile to text no tenant gate or lineage pass can read.
    #[test]
    fn a_no_select_cast_or_rename_statement_parses_and_names_its_input() {
        let dialect = sqlparser::dialect::ClickHouseDialect {};
        for transforms in [
            vec!["cast(amount,Float64)"],
            vec!["rename(amount,amount_usd)"],
            vec!["rename(amount,amount_usd)", "cast(amount,Float64)"],
        ] {
            let sql = render_from("orders", &transforms);
            assert_eq!(
                crate::sql_rewrite::referenced_tables(&sql, &dialect),
                Some(vec!["orders".to_owned()]),
                "statement parses and reads its input: {sql}"
            );
        }
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
    /// R1-S4: the whole text, not a suffix — the SQL goes in between line
    /// breaks so the comment cannot swallow the closing bracket.
    #[test]
    fn a_sql_step_ending_in_a_line_comment_keeps_its_closing_bracket() {
        let steps = vec![sql_step("selected", "SELECT id FROM orders -- keep this")];
        assert_eq!(
            compile(&one_input(), &steps, None, Some("icecat_api")).unwrap(),
            "WITH\n  `orders` AS (SELECT * FROM silver.`orders`),\n  `selected` AS (\n\
             SELECT id FROM orders -- keep this\n)\nSELECT * FROM `selected`"
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

    /// R1-S4: pinned whole — the prefix of the steps, then the one named.
    #[test]
    fn up_to_a_middle_step_stops_there() {
        let inputs = vec![input("orders", "silver", "orders")];
        let steps = vec![
            sql_step("first", "SELECT id FROM orders"),
            sql_step("second", "SELECT id FROM first"),
            sql_step("third", "SELECT id FROM second"),
        ];
        assert_eq!(
            compile(&inputs, &steps, Some("second"), Some("icecat_api")).unwrap(),
            "WITH\n  `orders` AS (SELECT * FROM silver.`orders`),\n  `first` AS (\n\
             SELECT id FROM orders\n),\n  `second` AS (\nSELECT id FROM first\n)\n\
             SELECT * FROM `second`"
        );
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
    fn an_unsafe_zone_is_refused_at_its_input() {
        let inputs = vec![input("orders", "sil ver", "orders")];
        let err = refusal(&inputs, &[]);
        assert_eq!(err.path(), "inputs[0]");
        assert_eq!(err.message(), super::INVALID_TABLE);
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

    // BLD-11 review R5: a rename that lands on a name this step also emits
    // would fail the next run on a duplicate column; refuse it at save.
    #[test]
    fn a_second_rename_to_the_same_name_is_refused_at_save() {
        let steps = vec![transform_step(
            "dup",
            "orders",
            &["rename(a,b)", "rename(c,b)"],
        )];
        let err = refusal(&one_input(), &steps);
        assert_eq!(err.path(), "steps[0]");
        assert_eq!(err.message(), super::DUPLICATE_OUTPUT_COLUMN);
    }

    #[test]
    fn a_rename_onto_another_selected_column_is_refused_at_save() {
        let steps = vec![transform_step(
            "dup",
            "orders",
            &["select(a,b)", "rename(a,b)"],
        )];
        let err = refusal(&one_input(), &steps);
        assert_eq!(err.path(), "steps[0]");
        assert_eq!(err.message(), super::DUPLICATE_OUTPUT_COLUMN);
    }

    #[test]
    fn a_single_rename_still_compiles() {
        let steps = vec![transform_step("ok", "orders", &["rename(a,b)"])];
        assert!(compile(&one_input(), &steps, None, Some("icecat_api")).is_ok());
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

    /// R2-B1: the reference is read AS WRITTEN. `ClickHouse` matches a `WITH`
    /// name by exact case, so `FROM ORDERS` is not the input `orders` — it is a
    /// table in the connection's own database, which neither the gate nor the
    /// rewrite sees.
    #[test]
    fn a_read_that_differs_only_in_case_is_refused_the_other_way_round() {
        let inputs = vec![input("orders", "silver", "orders")];
        let steps = vec![sql_step("copied", "SELECT * FROM ORDERS")];
        let err = refusal(&inputs, &steps);
        assert_eq!(err.path(), "steps[0]");
        assert_eq!(err.message(), super::READS_NOT_INPUT);
    }

    /// R2-B1: an input whose name has a capital letter can be read, exactly.
    #[test]
    fn an_input_with_a_capital_letter_is_read_exactly() {
        let inputs = vec![input("Orders", "silver", "orders")];
        let steps = vec![sql_step("copied", "SELECT * FROM Orders")];
        assert_eq!(
            compile(&inputs, &steps, None, Some("icecat_api")).unwrap(),
            "WITH\n  `Orders` AS (SELECT * FROM silver.`orders`),\n  `copied` AS (\n\
             SELECT * FROM Orders\n)\nSELECT * FROM `copied`"
        );
    }

    /// R2-B1: the step's OWN `WITH` names are read as written too — `t` is the
    /// alias, `T` is a table nobody declared.
    #[test]
    fn a_steps_own_with_name_is_matched_as_written() {
        let inputs = vec![input("orders", "silver", "orders")];
        let accepted = vec![sql_step(
            "aggregated",
            "WITH t AS (SELECT id FROM orders) SELECT * FROM t",
        )];
        assert!(compile(&inputs, &accepted, None, Some("icecat_api")).is_ok());

        let refused_steps = vec![sql_step(
            "aggregated",
            "WITH t AS (SELECT id FROM orders) SELECT * FROM T",
        )];
        let err = refusal(&inputs, &refused_steps);
        assert_eq!(err.path(), "steps[0]");
        assert_eq!(err.message(), super::READS_NOT_INPUT);
    }

    /// R2-B1: a name inside a subquery or a join is checked the same way.
    #[test]
    fn a_name_inside_a_subquery_or_a_join_is_checked_too() {
        let inputs = vec![input("orders", "silver", "orders")];
        for sql in [
            "SELECT * FROM orders WHERE id IN (SELECT id FROM ORDERS)",
            "SELECT * FROM orders AS o JOIN ORDERS AS x ON o.id = x.id",
        ] {
            let steps = vec![sql_step("copied", sql)];
            let err = refusal(&inputs, &steps);
            assert_eq!(err.path(), "steps[0]", "{sql}");
            assert_eq!(err.message(), super::READS_NOT_INPUT, "{sql}");
        }
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

    /// D4a: `up_to` follows the same rule — up to the input of a today's-shape
    /// definition is the plain read of its table, with no `WITH`.
    #[test]
    fn up_to_the_input_of_todays_shape_is_the_plain_read() {
        let steps = vec![transform_step("transformed", "orders", &["select(id)"])];
        assert_eq!(
            compile(&one_input(), &steps, Some("orders"), Some("icecat_api")).unwrap(),
            "SELECT * FROM silver.`orders`"
        );
        assert_eq!(
            compile(
                &one_input(),
                &steps,
                Some("transformed"),
                Some("icecat_api")
            )
            .unwrap(),
            "SELECT id FROM silver.`orders`"
        );
    }

    #[test]
    fn the_legacy_predicate_is_exactly_one_input_and_one_step_reading_it() {
        let inputs = vec![input("orders", "silver", "orders")];
        assert!(super::is_legacy_shape(&inputs, &[]));
        assert!(super::is_legacy_shape(
            &inputs,
            &[transform_step("transformed", "orders", &["select(id)"])]
        ));
        // A step reading something else, a SQL step and a second input are
        // not today's shape.
        assert!(!super::is_legacy_shape(
            &inputs,
            &[transform_step("transformed", "other", &["select(id)"])]
        ));
        assert!(!super::is_legacy_shape(
            &inputs,
            &[sql_step("copied", "SELECT id FROM orders")]
        ));
        assert!(!super::is_legacy_shape(
            &[input("a", "silver", "a"), input("b", "silver", "b")],
            &[]
        ));
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

    // ── BLD-16 (BLD-1 part 2): the three form kinds, and a step as SQL ──

    use lakehouse_store::pipelines::{JoinColumn, JoinPair, Measure, StepCondition};

    /// J2's join step, exactly as the plan's contract section spells it.
    fn j2_join_step() -> lakehouse_store::pipelines::PipelineStep {
        lakehouse_store::pipelines::PipelineStep::Join {
            name: "joined".to_owned(),
            left: "orders".to_owned(),
            right: "customers".to_owned(),
            join_type: "left".to_owned(),
            on: vec![JoinPair {
                left: "customer_id".to_owned(),
                right: "id".to_owned(),
            }],
            columns: vec![
                JoinColumn {
                    side: "left".to_owned(),
                    column: "order_id".to_owned(),
                    name: String::new(),
                },
                JoinColumn {
                    side: "left".to_owned(),
                    column: "amount".to_owned(),
                    name: String::new(),
                },
                JoinColumn {
                    side: "right".to_owned(),
                    column: "country".to_owned(),
                    name: String::new(),
                },
                JoinColumn {
                    side: "right".to_owned(),
                    column: "id".to_owned(),
                    name: "customer_ref".to_owned(),
                },
            ],
        }
    }

    /// J2's aggregate step, reading the join above.
    fn j2_aggregate_step() -> lakehouse_store::pipelines::PipelineStep {
        lakehouse_store::pipelines::PipelineStep::Aggregate {
            name: "by_country".to_owned(),
            reads: "joined".to_owned(),
            group_by: vec!["country".to_owned()],
            measures: vec![
                Measure {
                    function: "sum".to_owned(),
                    column: "amount".to_owned(),
                    name: "revenue".to_owned(),
                },
                Measure {
                    function: "count".to_owned(),
                    column: String::new(),
                    name: "order_count".to_owned(),
                },
            ],
            having_match: "all".to_owned(),
            having: vec![StepCondition {
                column: "revenue".to_owned(),
                operator: ">".to_owned(),
                value: "1000".to_owned(),
            }],
        }
    }

    /// J2's filter step, reading the aggregate above.
    fn j2_filter_step() -> lakehouse_store::pipelines::PipelineStep {
        lakehouse_store::pipelines::PipelineStep::Filter {
            name: "big_or_local".to_owned(),
            reads: "by_country".to_owned(),
            match_mode: "any".to_owned(),
            conditions: vec![
                StepCondition {
                    column: "revenue".to_owned(),
                    operator: ">=".to_owned(),
                    value: "5000".to_owned(),
                },
                StepCondition {
                    column: "country".to_owned(),
                    operator: "=".to_owned(),
                    value: "ID".to_owned(),
                },
            ],
        }
    }

    /// The body of one step's CTE entry in a compiled statement: the text
    /// between ``  `name` AS ( `` and the closing bracket of that line. The
    /// entries are joined with `,\n`, so a non-final line ends with `),` —
    /// the comma is the join's, not the body's.
    fn cte_body(compiled: &str, name: &str) -> String {
        let prefix = format!("  `{name}` AS (");
        let line = compiled
            .lines()
            .find(|line| line.starts_with(&prefix))
            .expect("the step has a CTE entry");
        let body = line
            .strip_prefix(&prefix)
            .and_then(|body| body.strip_suffix(','))
            .or_else(|| line.strip_prefix(&prefix))
            .and_then(|body| body.strip_suffix(')'))
            .expect("the entry opens after the name and closes at the line's end");
        body.to_owned()
    }

    /// One statement, parsed and re-rendered by `sqlparser`, so only a real
    /// difference in the query — not the line breaks a SQL step's text gets
    /// and a form step's does not — can fail the comparison.
    fn re_rendered(sql: &str) -> String {
        use sqlparser::ast::Statement;
        let dialect = sqlparser::dialect::ClickHouseDialect {};
        let parsed = sqlparser::parser::Parser::parse_sql(&dialect, sql).expect("it parses");
        let [Statement::Query(_)] = parsed.as_slice() else {
            panic!("one query, not: {sql}");
        };
        parsed[0].to_string()
    }

    #[test]
    fn a_join_step_compiles_into_the_with_form() {
        let join_body = "SELECT `__left`.`order_id` AS `order_id`, `__left`.`amount` AS `amount`, \
                         `__right`.`country` AS `country`, `__right`.`id` AS `customer_ref` \
                         FROM `orders` AS `__left` LEFT JOIN `customers` AS `__right` \
                         ON `__left`.`customer_id` = `__right`.`id`";
        assert_eq!(
            compile(&two_inputs(), &[j2_join_step()], None, Some("icecat_api")).unwrap(),
            format!(
                "WITH\n  `orders` AS (SELECT * FROM silver.`orders`),\n  \
                 `customers` AS (SELECT * FROM silver.`customers`),\n  \
                 `joined` AS ({join_body})\nSELECT * FROM `joined`"
            )
        );
    }

    /// J2's three steps, one after a two-input definition: the whole
    /// statement, pinned. A form step's entry is one line, the shape a
    /// `transform` step's already has (J6).
    #[test]
    fn j2s_three_steps_compile_into_one_statement() {
        let steps = vec![j2_join_step(), j2_aggregate_step(), j2_filter_step()];
        let compiled = compile(&two_inputs(), &steps, None, Some("icecat_api")).unwrap();
        assert_eq!(
            compiled,
            "WITH\n  `orders` AS (SELECT * FROM silver.`orders`),\n  \
             `customers` AS (SELECT * FROM silver.`customers`),\n  \
             `joined` AS (SELECT `__left`.`order_id` AS `order_id`, `__left`.`amount` AS \
             `amount`, `__right`.`country` AS `country`, `__right`.`id` AS `customer_ref` \
             FROM `orders` AS `__left` LEFT JOIN `customers` AS `__right` \
             ON `__left`.`customer_id` = `__right`.`id`),\n  \
             `by_country` AS (SELECT `country`, sum(`amount`) AS `revenue`, count() AS \
             `order_count` FROM `joined` GROUP BY `country` HAVING sum(`amount`) > 1000),\n  \
             `big_or_local` AS (SELECT * FROM `by_country` WHERE `revenue` >= '5000' OR \
             `country` = 'ID')\n\
             SELECT * FROM `big_or_local`"
        );
    }

    /// D6a for the new kinds: a join's `left`/`right` name what the step
    /// reads, and the refusal names the FIELD, unlike a `transform` step's
    /// `steps[i]`.
    #[test]
    fn a_join_reading_a_later_step_is_refused_at_its_left_or_right() {
        for field in ["left", "right"] {
            let join = lakehouse_store::pipelines::PipelineStep::Join {
                name: "j".to_owned(),
                left: if field == "left" {
                    "later".to_owned()
                } else {
                    "orders".to_owned()
                },
                right: if field == "right" {
                    "later".to_owned()
                } else {
                    "orders".to_owned()
                },
                join_type: "inner".to_owned(),
                on: vec![JoinPair {
                    left: "a".to_owned(),
                    right: "b".to_owned(),
                }],
                columns: vec![JoinColumn {
                    side: "left".to_owned(),
                    column: "a".to_owned(),
                    name: String::new(),
                }],
            };
            let steps = vec![join, sql_step("later", "SELECT id FROM orders")];
            let err = refusal(&two_inputs(), &steps);
            assert_eq!(err.path(), format!("steps[0].{field}"), "{field}");
            assert_eq!(err.message(), super::READS_LATER_STEP, "{field}");
        }
    }

    #[test]
    fn a_join_reading_a_name_in_another_case_is_refused() {
        let join = lakehouse_store::pipelines::PipelineStep::Join {
            name: "j".to_owned(),
            left: "Orders".to_owned(),
            right: "customers".to_owned(),
            join_type: "inner".to_owned(),
            on: vec![JoinPair {
                left: "a".to_owned(),
                right: "b".to_owned(),
            }],
            columns: vec![JoinColumn {
                side: "left".to_owned(),
                column: "a".to_owned(),
                name: String::new(),
            }],
        };
        let err = refusal(&two_inputs(), &[join]);
        assert_eq!(err.path(), "steps[0].left");
        assert_eq!(err.message(), super::READS_NOT_INPUT);
    }

    #[test]
    fn an_aggregate_reading_an_unknown_name_is_refused_at_reads() {
        let err = refusal(
            &one_input(),
            &[lakehouse_store::pipelines::PipelineStep::Aggregate {
                name: "a".to_owned(),
                reads: "nope".to_owned(),
                group_by: vec!["c".to_owned()],
                measures: Vec::new(),
                having_match: String::new(),
                having: Vec::new(),
            }],
        );
        assert_eq!(err.path(), "steps[0].reads");
        assert_eq!(err.message(), super::READS_NOT_INPUT);
    }

    /// J7: a pipeline with a new kind is not today's shape, so it compiles
    /// to the `WITH` form and is not noted for column lineage (D4b).
    #[test]
    fn the_legacy_predicate_is_false_for_join_aggregate_and_filter_steps() {
        let inputs = one_input();
        let read_input = lakehouse_store::pipelines::PipelineStep::Aggregate {
            name: "a".to_owned(),
            reads: "orders".to_owned(),
            group_by: vec!["c".to_owned()],
            measures: Vec::new(),
            having_match: String::new(),
            having: Vec::new(),
        };
        assert!(!super::is_legacy_shape(&inputs, &[j2_join_step()]));
        assert!(!super::is_legacy_shape(&inputs, &[read_input]));
        assert!(!super::is_legacy_shape(
            &inputs,
            &[lakehouse_store::pipelines::PipelineStep::Filter {
                name: "f".to_owned(),
                reads: "a".to_owned(),
                match_mode: String::new(),
                conditions: vec![StepCondition {
                    column: "c".to_owned(),
                    operator: "=".to_owned(),
                    value: "1".to_owned(),
                }],
            }]
        ));
    }

    #[test]
    fn up_to_a_join_step_stops_there() {
        let steps = vec![j2_join_step(), j2_aggregate_step()];
        let compiled = compile(&two_inputs(), &steps, Some("joined"), Some("icecat_api")).unwrap();
        assert!(compiled.ends_with("SELECT * FROM `joined`"), "{compiled}");
        assert!(!compiled.contains("`by_country`"), "{compiled}");
    }

    /// J6: for each of the four form kinds, the step-as-SQL answer IS the
    /// body the compiler put inside that step's `WITH` entry — the same
    /// [`step_body`] produced both, so they cannot differ.
    #[test]
    fn step_sql_of_a_form_step_is_its_with_body() {
        let steps = vec![
            transform_step("paid", "orders", &["select(order_id,amount,status)"]),
            j2_join_step(),
            j2_aggregate_step(),
            j2_filter_step(),
        ];
        let compiled = compile(&two_inputs(), &steps, None, Some("icecat_api")).unwrap();
        for name in ["paid", "joined", "by_country", "big_or_local"] {
            assert_eq!(
                super::step_sql(&two_inputs(), &steps, name).unwrap(),
                cte_body(&compiled, name),
                "{name}"
            );
        }
    }

    /// J6, the promise behind "Switch to SQL step": replacing a form step
    /// with a SQL step that carries [`step_sql`]'s answer compiles to the
    /// SAME statement, so the switch changes what a person can edit, not
    /// what a run computes.
    #[test]
    fn switching_a_form_step_to_sql_compiles_to_the_same_statement() {
        let steps = vec![
            transform_step("paid", "orders", &["select(order_id,amount,status)"]),
            j2_join_step(),
            j2_aggregate_step(),
            j2_filter_step(),
        ];
        let original = compile(&two_inputs(), &steps, None, Some("icecat_api")).unwrap();
        for (index, name) in ["paid", "joined", "by_country", "big_or_local"]
            .iter()
            .enumerate()
        {
            let sql = super::step_sql(&two_inputs(), &steps, name).unwrap();
            let mut switched = steps.clone();
            switched[index] = sql_step(name, &sql);
            let other =
                validate_and_compile(&two_inputs(), &switched, None, Some("icecat_api")).unwrap();
            assert_eq!(
                re_rendered(&original),
                re_rendered(&other),
                "the switch of {name} changes the statement"
            );
        }
    }

    #[test]
    fn step_sql_of_an_input_name_is_refused() {
        let err = super::step_sql(&two_inputs(), &[], "orders").unwrap_err();
        assert_eq!(err.path(), "step");
        assert_eq!(err.message(), super::STEP_UNKNOWN);
    }

    #[test]
    fn step_sql_of_an_unknown_name_is_refused() {
        let err = super::step_sql(&two_inputs(), &[j2_join_step()], "nope").unwrap_err();
        assert_eq!(err.path(), "step");
        assert_eq!(err.message(), super::STEP_UNKNOWN);
    }

    /// A broken step BEFORE the named one is refused at its own index — the
    /// answer for a later step cannot lean on an earlier one that would not
    /// compile.
    #[test]
    fn step_sql_refuses_a_bad_earlier_step() {
        let broken = lakehouse_store::pipelines::PipelineStep::Join {
            name: "broken".to_owned(),
            left: "orders".to_owned(),
            right: "customers".to_owned(),
            join_type: "cross".to_owned(),
            on: vec![JoinPair {
                left: "a".to_owned(),
                right: "b".to_owned(),
            }],
            columns: vec![JoinColumn {
                side: "left".to_owned(),
                column: "a".to_owned(),
                name: String::new(),
            }],
        };
        let steps = vec![broken, j2_aggregate_step()];
        let err = super::step_sql(&two_inputs(), &steps, "by_country").unwrap_err();
        assert_eq!(err.path(), "steps[0].joinType");
        assert_eq!(err.message(), "a join is inner, left, right or full");
    }
}
