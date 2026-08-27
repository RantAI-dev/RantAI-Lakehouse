//! `POST /api/embed/data`, `GET /api/public/dashboard/{token}` — read-only
//! dashboard views for external viewers (signed embed & public share link).
//!
//! Ports `src/app/api/embed/data/route.ts` and
//! `src/app/api/public/dashboard/[token]/route.ts`. Both assemble the same
//! `{ board, layout, charts, results }` payload as `dashboard::get`, but
//! scoped to one board and with dashboard-wide filters *locked* (a signed
//! embed's JWT `params`, or a public board's own stored filters) rather
//! than caller-supplied.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use lakehouse_bi::builder::sql_with_filters;
use lakehouse_bi::store::{self, Board, FilterDef, StoredChartSpec};
use lakehouse_clickhouse::ChClient;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::json::ApiJson;
use crate::routes::support::{mart_columns, render_stored_spec, run_spec_sql};
use crate::state::AppState;

/// `{ jwt }` — the `POST /api/embed/data` body shape.
#[derive(Debug, Default, Deserialize)]
struct EmbedDataBody {
    #[serde(default)]
    jwt: Option<String>,
}

/// `POST /api/embed/data` — signed-embed (Metabase-style) dashboard data.
pub async fn data(State(state): State<AppState>, body: Bytes) -> Response {
    // `try { jwt = String((await req.json())?.jwt ?? "") } catch { /* ignore */ }`
    // — an unparseable body is swallowed, not a 400; it just yields an
    // empty jwt, which the emptiness check below rejects the normal way.
    let jwt = serde_json::from_slice::<EmbedDataBody>(&body)
        .ok()
        .and_then(|b| b.jwt)
        .filter(|s| !s.is_empty());
    let Some(jwt) = jwt else {
        return (
            StatusCode::BAD_REQUEST,
            ApiJson(json!({ "error": "jwt wajib" })),
        )
            .into_response();
    };

    let secret = match state.embed_secret.get_embed_secret().await {
        Ok(s) => s,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiJson(json!({ "error": err.to_string() })),
            )
                .into_response();
        }
    };
    let Some(claims) = lakehouse_embed::verify_embed(&jwt, &secret) else {
        return (
            StatusCode::UNAUTHORIZED,
            ApiJson(json!({ "error": "invalid_or_expired" })),
        )
            .into_response();
    };

    let Some(board_id) = claims.resource.and_then(|r| r.dashboard) else {
        return (
            StatusCode::BAD_REQUEST,
            ApiJson(json!({ "error": "no_resource" })),
        )
            .into_response();
    };

    let board = match store::get_board(&state.clickhouse, &board_id).await {
        Ok(b) => b,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiJson(json!({ "error": err.to_string() })),
            )
                .into_response();
        }
    };
    let Some(board) = board.filter(|b| b.embed_enabled.unwrap_or(false)) else {
        return (
            StatusCode::FORBIDDEN,
            ApiJson(json!({ "error": "embedding_disabled" })),
        )
            .into_response();
    };

    let mut filters = board.filters.clone().unwrap_or_default();
    filters.extend(params_to_filters(claims.params));

    match render_board_payload(&state.clickhouse, &board, &board_id, &filters).await {
        Ok(body) => (StatusCode::OK, ApiJson(body)).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            ApiJson(json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

/// `paramsToFilters` in `embed/data/route.ts`: a signed embed's locked JWT
/// `params` (`{ col: val | [vals] }`) become dashboard filters the viewer
/// cannot override.
fn params_to_filters(params: Option<std::collections::HashMap<String, Value>>) -> Vec<FilterDef> {
    let Some(params) = params else {
        return Vec::new();
    };
    params
        .into_iter()
        .map(|(column, v)| {
            let values = match v {
                Value::Array(items) => items.iter().map(value_to_string).collect(),
                other => vec![value_to_string(&other)],
            };
            FilterDef { column, values }
        })
        .collect()
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// `GET /api/public/dashboard/{token}` — read-only public share view.
pub async fn public_dashboard(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> Response {
    let board = match store::get_board_by_token(&state.clickhouse, &token).await {
        Ok(b) => b,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiJson(json!({ "error": err.to_string() })),
            )
                .into_response();
        }
    };
    let Some(board) = board else {
        return (
            StatusCode::NOT_FOUND,
            ApiJson(json!({ "error": "not_found" })),
        )
            .into_response();
    };

    let filters = board.filters.clone().unwrap_or_default();
    let board_id = board.id.clone();
    match render_board_payload(&state.clickhouse, &board, &board_id, &filters).await {
        Ok(body) => (StatusCode::OK, ApiJson(body)).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            ApiJson(json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

/// Shared `{ board, layout, charts, results }` assembly for both
/// [`data`] and [`public_dashboard`]: every stored chart belonging to
/// `board_id`, filtered by `filters` (never by caller-supplied years —
/// neither route accepts a `year` parameter).
async fn render_board_payload(
    ch: &ChClient,
    board: &Board,
    board_id: &str,
    filters: &[FilterDef],
) -> Result<Value, lakehouse_clickhouse::ChError> {
    let stored = store::list_stored_charts(ch).await?;
    let stored_for_board: Vec<&StoredChartSpec> = stored
        .iter()
        .filter(|c| {
            if c.board.is_empty() {
                board_id == "default"
            } else {
                c.board == board_id
            }
        })
        .collect();

    let need_cols = filters.iter().any(|f| !f.values.is_empty());
    let cols = if need_cols {
        mart_columns(ch).await?
    } else {
        std::collections::HashMap::new()
    };

    let mut results = serde_json::Map::new();
    let mut charts_out = Vec::with_capacity(stored_for_board.len());
    for c in &stored_for_board {
        let sql = sql_with_filters(c, &[], filters, &cols);
        let (id, val) = run_spec_sql(ch, &c.spec.id, &sql).await;
        results.insert(id, val);

        let mut rendered = render_stored_spec(&c.spec, c.source);
        rendered["board"] = json!(c.board);
        rendered["def"] = serde_json::to_value(&c.def).unwrap_or_else(|_| json!({}));
        charts_out.push(rendered);
    }

    let layout = board.layout.as_ref().map_or_else(
        || json!({}),
        |l| {
            let mut m = serde_json::Map::new();
            for (k, b) in l {
                m.insert(k.clone(), json!({ "x": b.x, "y": b.y, "w": b.w, "h": b.h }));
            }
            Value::Object(m)
        },
    );

    Ok(json!({
        "board": { "id": board.id, "name": board.name },
        "layout": layout,
        "charts": charts_out,
        "results": results,
    }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn params_to_filters_none_yields_empty() {
        assert!(params_to_filters(None).is_empty());
    }

    #[test]
    fn params_to_filters_wraps_single_value_in_array() {
        let mut params = std::collections::HashMap::new();
        params.insert("kawasan".to_owned(), json!("Asia"));
        let filters = params_to_filters(Some(params));
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].column, "kawasan");
        assert_eq!(filters[0].values, vec!["Asia".to_owned()]);
    }

    #[test]
    fn params_to_filters_passes_through_array_values() {
        let mut params = std::collections::HashMap::new();
        params.insert("tahun".to_owned(), json!(["2023", "2024"]));
        let filters = params_to_filters(Some(params));
        assert_eq!(
            filters[0].values,
            vec!["2023".to_owned(), "2024".to_owned()]
        );
    }
}
