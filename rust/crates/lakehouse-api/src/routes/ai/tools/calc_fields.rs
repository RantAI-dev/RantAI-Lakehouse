//! Calculated-field tools (`BI-8`, with `AI-4`), and the value-group and
//! hierarchy tools of `BI-10` that ride the same routes: `list_formula_functions`,
//! `list_calculated_fields`, `validate_formula`, `create_calculated_field`,
//! `update_calculated_field` and `delete_calculated_field`.
//!
//! Each one calls the console route's own handler (`routes::calc_fields`)
//! and reads its JSON back, so the assistant cannot reach anything the
//! console cannot: the same formula checker, the same name rules, the same
//! refusal while a chart uses a field. The assistant writes a *formula* in
//! the product's own language, never SQL; the server compiles it.

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Query, State};
use lakehouse_auth::Principal;
use serde_json::{Map, Value, json};

use super::api_result_to_value;
use crate::routes::{calc_fields, hierarchies, params};
use crate::state::AppState;

/// The tool argument `sqlSource` is the route's `source`.
fn route_args(args: &Map<String, Value>) -> Map<String, Value> {
    let mut out = args.clone();
    if let Some(source) = out.remove("sqlSource") {
        out.insert("source".to_owned(), source);
    }
    out
}

fn body(args: &Map<String, Value>) -> Bytes {
    Bytes::from(serde_json::to_vec(&Value::Object(route_args(args))).unwrap_or_default())
}

/// The formula functions the assistant reads: one entry per category, in
/// the order the catalog has them, each function one line of its signature
/// and its help.
///
/// The examples, the arity and the flags are left out: the chat passes at
/// most 8,000 characters of a tool result to the model
/// (`src/routes/ai/mod.rs:1067`), and the whole list has to fit under that
/// cut or the model never reads the end of it.
fn functions_by_category() -> Vec<Value> {
    let mut grouped: Vec<(&str, Vec<String>)> = Vec::new();
    for f in lakehouse_bi::formula::catalog::CATALOG {
        let line = format!("{}: {}", f.signature, f.help);
        match grouped
            .iter()
            .position(|(category, _)| *category == f.category)
        {
            Some(index) => grouped[index].1.push(line),
            None => grouped.push((f.category, vec![line])),
        }
    }
    grouped
        .into_iter()
        .map(|(category, list)| json!({ "category": category, "list": list }))
        .collect()
}

pub(super) fn list_formula_functions() -> Value {
    json!({
        "language": "[Column Name] or a bare name for a column or another calculated field of the same source; numbers; 'text' or \"text\"; + - * /; = != < <= > >=; and, or, not; parentheses; the functions below (names are not case sensitive). A formula is either one value per row, for example [revenue] - [cost], or an aggregate, for example Sum([revenue]) / CountDistinct([customer]); the two cannot be mixed in one formula except through an aggregate. Table calculations (RunningTotal, Rank, PercentOfTotal, Offset, MovingAverage) and period comparisons (PreviousPeriod, SamePeriodLastYear) take an aggregate and are computed over the chart's result in its dimension order; they can only be a measure. Fixed([column], aggregate) computes an aggregate at columns the chart groups by.",
        "functions": functions_by_category(),
    })
}

pub(super) async fn list_calculated_fields(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let q = match serde_json::from_value(Value::Object(route_args(args))) {
        Ok(q) => q,
        Err(_unparsed) => return json!({ "error": "give mart or sqlSource" }),
    };
    let who = principal.cloned().map(Extension);
    api_result_to_value(calc_fields::list(State(state.clone()), who, Query(q)).await).await
}

pub(super) async fn validate_formula(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let who = principal.cloned().map(Extension);
    api_result_to_value(calc_fields::validate(State(state.clone()), who, body(args)).await).await
}

/// The route answers `{ ok, field }`; the audit trail and the model read the
/// field's id at the top, as for every other tool that creates something.
fn with_id(mut result: Value) -> Value {
    if let Some(id) = result.pointer("/field/id").cloned() {
        result["id"] = id;
    }
    result
}

/// BI-8 review fix (SHOULD-FIX) R3: the field is recorded under the person
/// who asked the assistant, as the HTTP route records it (display only; it
/// grants nothing). With no principal (a call outside a session) it stays
/// empty.
pub(super) async fn create_calculated_field(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let who = principal.cloned().map(Extension);
    with_id(
        api_result_to_value(calc_fields::create(State(state.clone()), who, body(args)).await).await,
    )
}

pub(super) async fn update_calculated_field(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let who = principal.cloned().map(Extension);
    with_id(
        api_result_to_value(calc_fields::update(State(state.clone()), who, body(args)).await).await,
    )
}

pub(super) async fn delete_calculated_field(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let q = match serde_json::from_value(Value::Object(route_args(args))) {
        Ok(q) => q,
        Err(_unparsed) => return json!({ "error": "id is required" }),
    };
    let who = principal.cloned().map(Extension);
    api_result_to_value(calc_fields::delete(State(state.clone()), who, Query(q)).await).await
}

/// `create_value_group` (`BI-10`): the calculated-field route with a `group`
/// in place of a formula, so the same name rules, permissions and
/// author recording apply.
pub(super) async fn create_value_group(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    create_calculated_field(state, principal, args).await
}

/// The hierarchy tools call the console route's own handlers
/// (`routes::hierarchies`), as the calculated-field tools do.
pub(super) async fn list_hierarchies(state: &AppState, args: &Map<String, Value>) -> Value {
    let q = match serde_json::from_value(Value::Object(route_args(args))) {
        Ok(q) => q,
        Err(_unparsed) => return json!({ "error": "give mart or sqlSource" }),
    };
    api_result_to_value(hierarchies::list(State(state.clone()), Query(q)).await).await
}

pub(super) async fn create_hierarchy(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let who = principal.cloned().map(Extension);
    let result =
        api_result_to_value(hierarchies::create(State(state.clone()), who, body(args)).await).await;
    match result.pointer("/hierarchy/id").cloned() {
        Some(id) => {
            let mut out = result;
            out["id"] = id;
            out
        }
        None => result,
    }
}

pub(super) async fn delete_hierarchy(state: &AppState, args: &Map<String, Value>) -> Value {
    let q = match serde_json::from_value(Value::Object(route_args(args))) {
        Ok(q) => q,
        Err(_unparsed) => return json!({ "error": "id is required" }),
    };
    api_result_to_value(hierarchies::delete(State(state.clone()), Query(q)).await).await
}

/// The route answers `{ ok, param }`; the audit trail and the model read the
/// id at the top, like a created field's.
fn with_param_id(mut result: Value) -> Value {
    if let Some(id) = result.pointer("/param/id").cloned() {
        result["id"] = id;
    }
    result
}

/// Whether `name` is one of the fourteen tools [`run`] dispatches (one guard arm in
/// `run_tool`, which is at clippy's line limit).
pub(super) fn handles(name: &str) -> bool {
    // BI-2: the metric tools ride the same single guard arm in `run_tool`.
    super::metrics::handles(name)
        // BI-18 part C: the tab tools ride the same guard arm.
        || super::tabs::handles(name)
        // BI-16 part C2: the map list rides it too.
        || super::maps::handles(name)
        || matches!(
            name,
            "list_formula_functions"
                | "list_calculated_fields"
                | "validate_formula"
                | "create_calculated_field"
                | "update_calculated_field"
                | "delete_calculated_field"
                | "create_value_group"
                | "list_hierarchies"
                | "create_hierarchy"
                | "delete_hierarchy"
                | "list_parameters"
                | "create_parameter"
                | "update_parameter"
                | "delete_parameter"
        )
}

/// Dispatch one of the fourteen tools by name (one arm in `run_tool`, which is at
/// clippy's line limit).
pub(super) async fn run(
    state: &AppState,
    principal: Option<&Principal>,
    name: &str,
    args: &Map<String, Value>,
) -> Value {
    if super::metrics::handles(name) {
        return super::metrics::run(state, principal, name, args).await;
    }
    if super::tabs::handles(name) {
        return super::tabs::run(state, principal, name, args).await;
    }
    if super::maps::handles(name) {
        return super::maps::list_maps(state).await;
    }
    match name {
        "list_formula_functions" => list_formula_functions(),
        "list_calculated_fields" => list_calculated_fields(state, principal, args).await,
        "validate_formula" => validate_formula(state, principal, args).await,
        "create_calculated_field" => create_calculated_field(state, principal, args).await,
        "update_calculated_field" => update_calculated_field(state, principal, args).await,
        "delete_calculated_field" => delete_calculated_field(state, principal, args).await,
        "create_value_group" => create_value_group(state, principal, args).await,
        "list_hierarchies" => list_hierarchies(state, args).await,
        "create_hierarchy" => create_hierarchy(state, principal, args).await,
        "delete_hierarchy" => delete_hierarchy(state, args).await,
        "list_parameters" => api_result_to_value(params::list(State(state.clone())).await).await,
        "create_parameter" => {
            let who = principal.cloned().map(Extension);
            with_param_id(
                api_result_to_value(params::create(State(state.clone()), who, body(args)).await)
                    .await,
            )
        }
        "update_parameter" => {
            let who = principal.cloned().map(Extension);
            api_result_to_value(params::update(State(state.clone()), who, body(args)).await).await
        }
        "delete_parameter" => {
            let q = match serde_json::from_value(Value::Object(route_args(args))) {
                Ok(q) => q,
                Err(_unparsed) => return json!({ "error": "id is required" }),
            };
            let who = principal.cloned().map(Extension);
            api_result_to_value(params::delete(State(state.clone()), who, Query(q)).await).await
        }
        other => json!({ "error": format!("unknown tool: {other}") }),
    }
}

#[cfg(test)]
mod function_list_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use lakehouse_bi::formula::catalog::CATALOG;
    use serde_json::Value;

    /// The function list the assistant reads: one entry per category, one
    /// line per function.
    fn listed_by_category() -> Vec<Value> {
        let listed = super::list_formula_functions();
        listed["functions"]
            .as_array()
            .expect("the function list")
            .clone()
    }

    /// `BI-16` part C3 (T10): the function list the assistant reads has both
    /// spatial functions, with their help.
    #[test]
    fn list_formula_functions_lists_distance_and_region_of() {
        let functions = listed_by_category();
        let spatial = functions
            .iter()
            .find(|entry| entry["category"] == "spatial")
            .expect("the spatial category");
        let lines = spatial["list"].as_array().expect("the spatial functions");
        for name in ["Distance(", "RegionOf("] {
            assert!(
                lines
                    .iter()
                    .any(|line| line.as_str().is_some_and(|text| text.starts_with(name))),
                "{name} is not listed"
            );
        }
    }

    /// The chat passes at most 8,000 characters of a tool result to the
    /// model (`src/routes/ai/mod.rs:1067`), so the whole list has to fit
    /// under that cut: a list cut in the middle is a list the model never
    /// finishes reading.
    #[test]
    fn the_function_list_fits_the_tool_result_cut() {
        let listed = super::list_formula_functions();
        let serialised = serde_json::to_string(&listed).expect("the list is JSON");
        let length = serialised.chars().count();
        // The measured size, so a reader of a failed run knows how far the
        // list is from the bar.
        println!("the function list is {length} characters");
        assert!(
            length < 8_000,
            "the function list is {length} characters, the cut is 8,000"
        );
    }

    /// Every function of the catalog is listed once, under its own category.
    #[test]
    fn every_catalog_function_is_listed_once_under_its_own_category() {
        let functions = listed_by_category();
        let mut expected_categories: Vec<&str> = Vec::new();
        for f in CATALOG {
            if !expected_categories.contains(&f.category) {
                expected_categories.push(f.category);
            }
        }
        let categories: Vec<&str> = functions
            .iter()
            .map(|entry| entry["category"].as_str().expect("a category name"))
            .collect();
        assert_eq!(
            categories, expected_categories,
            "the categories are not the catalog's, in its order"
        );

        let mut lines_total = 0;
        for entry in &functions {
            let lines = entry["list"]
                .as_array()
                .expect("the functions of a category");
            assert!(
                !lines.is_empty(),
                "the category {} lists no function",
                entry["category"]
            );
            lines_total += lines.len();
        }
        assert_eq!(
            lines_total,
            CATALOG.len(),
            "the list has lines the catalog does not"
        );

        for f in CATALOG {
            let entry = functions
                .iter()
                .find(|entry| entry["category"] == f.category)
                .expect("the category of the function");
            let lines = entry["list"]
                .as_array()
                .expect("the functions of a category");
            let expected = format!("{}: {}", f.signature, f.help);
            let hits = lines
                .iter()
                .filter(|line| line.as_str() == Some(expected.as_str()))
                .count();
            assert_eq!(hits, 1, "{} is listed {hits} times", f.name);
        }
    }
}
