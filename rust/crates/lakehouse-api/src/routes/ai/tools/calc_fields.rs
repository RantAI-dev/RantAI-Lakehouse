//! Calculated-field tools (`BI-8`, with `AI-4`): `list_formula_functions`,
//! `list_calculated_fields`, `validate_formula`, `create_calculated_field`,
//! `update_calculated_field` and `delete_calculated_field`.
//!
//! Each one calls the console route's own handler (`routes::calc_fields`)
//! and reads its JSON back, so the assistant cannot reach anything the
//! console cannot: the same formula checker, the same name rules, the same
//! refusal while a chart uses a field. The assistant writes a *formula* in
//! the product's own language, never SQL; the server compiles it.

use axum::body::Bytes;
use axum::extract::{Query, State};
use serde_json::{Map, Value, json};

use super::api_result_to_value;
use crate::routes::calc_fields;
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

pub(super) fn list_formula_functions() -> Value {
    json!({
        "language": "[Column Name] or a bare name for a column or another calculated field of the same source; numbers; 'text' or \"text\"; + - * /; = != < <= > >=; and, or, not; parentheses; the functions below (names are not case sensitive). A formula is either one value per row, for example [revenue] - [cost], or an aggregate, for example Sum([revenue]) / CountDistinct([customer]); the two cannot be mixed in one formula except through an aggregate.",
        "functions": lakehouse_bi::formula::catalog::CATALOG,
    })
}

pub(super) async fn list_calculated_fields(state: &AppState, args: &Map<String, Value>) -> Value {
    let q = match serde_json::from_value(Value::Object(route_args(args))) {
        Ok(q) => q,
        Err(_unparsed) => return json!({ "error": "give mart or sqlSource" }),
    };
    api_result_to_value(calc_fields::list(State(state.clone()), Query(q)).await).await
}

pub(super) async fn validate_formula(state: &AppState, args: &Map<String, Value>) -> Value {
    api_result_to_value(calc_fields::validate(State(state.clone()), body(args)).await).await
}

/// The route answers `{ ok, field }`; the audit trail and the model read the
/// field's id at the top, as for every other tool that creates something.
fn with_id(mut result: Value) -> Value {
    if let Some(id) = result.pointer("/field/id").cloned() {
        result["id"] = id;
    }
    result
}

pub(super) async fn create_calculated_field(state: &AppState, args: &Map<String, Value>) -> Value {
    // `created_by` stays empty, as for the board tool: the assistant is not a
    // person.
    with_id(
        api_result_to_value(calc_fields::create(State(state.clone()), None, body(args)).await)
            .await,
    )
}

pub(super) async fn update_calculated_field(state: &AppState, args: &Map<String, Value>) -> Value {
    with_id(
        api_result_to_value(calc_fields::update(State(state.clone()), None, body(args)).await)
            .await,
    )
}

pub(super) async fn delete_calculated_field(state: &AppState, args: &Map<String, Value>) -> Value {
    let q = match serde_json::from_value(Value::Object(route_args(args))) {
        Ok(q) => q,
        Err(_unparsed) => return json!({ "error": "id is required" }),
    };
    api_result_to_value(calc_fields::delete(State(state.clone()), Query(q)).await).await
}
