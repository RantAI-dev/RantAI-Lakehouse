//! Dashboard/chart tools: `create_chart`, `update_chart`, `delete_chart`,
//! `create_board`, `list_boards`, `list_charts`, `suggest_dashboard`.
//! Moved out of `ai.rs` unchanged (T0.1 registry refactor).

use lakehouse_bi::specs::ChartSource;
use lakehouse_bi::store::{self, ChartInput};
use lakehouse_clickhouse::ChClient;
use serde_json::{Map, Value, json};

use super::arg_str;
use crate::routes::support::is_numeric_type;

/// Parse a tool call's raw args `Value` into a [`ChartInput`], the same way
/// `args as unknown as ChartInput` casts in `ai-tools.ts` (no validation at
/// this boundary — `spec_from_input` validates for real).
fn parse_chart_input(args: &Map<String, Value>) -> Result<ChartInput, String> {
    serde_json::from_value(Value::Object(args.clone())).map_err(|e| e.to_string())
}

pub(super) async fn create_chart(
    ch: &ChClient,
    args: &Map<String, Value>,
    id: Option<String>,
) -> Value {
    let input = match parse_chart_input(args) {
        Ok(i) => i,
        Err(e) => return json!({ "error": e }),
    };
    match store::spec_from_input(ch, &input, ChartSource::Ai, "ai", id).await {
        Ok(spec) => match store::insert_chart(ch, &spec).await {
            Ok(()) => json!({
                "created": true,
                "id": spec.spec.id,
                "title": spec.spec.title,
                "kind": spec.spec.kind,
                "mart": spec.spec.mart,
                "board": spec.board,
                "url": "/dashboards",
                "note": "Chart tersimpan & langsung tampil di halaman Dashboards.",
            }),
            Err(err) => json!({ "error": err.to_string() }),
        },
        Err(err) => json!({ "error": err.to_string() }),
    }
}

pub(super) async fn update_chart(ch: &ChClient, args: &Map<String, Value>) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id wajib" });
    }
    let input = match parse_chart_input(args) {
        Ok(i) => i,
        Err(e) => return json!({ "error": e }),
    };
    match store::spec_from_input(ch, &input, ChartSource::Ai, "ai", Some(id)).await {
        Ok(spec) => match store::insert_chart(ch, &spec).await {
            Ok(()) => json!({
                "updated": true,
                "id": spec.spec.id,
                "title": spec.spec.title,
                "kind": spec.spec.kind,
                "mart": spec.spec.mart,
            }),
            Err(err) => json!({ "error": err.to_string() }),
        },
        Err(err) => json!({ "error": err.to_string() }),
    }
}

pub(super) async fn create_board(ch: &ChClient, args: &Map<String, Value>) -> Value {
    match store::create_board(ch, &arg_str(args, "name")).await {
        Ok(board) => json!({
            "created": true, "id": board.id, "name": board.name,
            "note": "Pakai id ini di create_chart.board.",
        }),
        Err(err) => json!({ "error": err.to_string() }),
    }
}

pub(super) async fn list_boards(ch: &ChClient) -> Value {
    match store::list_boards(ch).await {
        Ok(boards) => {
            let mut out = vec![json!({ "id": "default", "name": "Main" })];
            out.extend(boards.iter().map(|b| json!({ "id": b.id, "name": b.name })));
            json!({ "boards": out })
        }
        Err(err) => json!({ "error": err.to_string() }),
    }
}

pub(super) async fn suggest_dashboard(ch: &ChClient) -> Value {
    let marts = match ch
        .rows(
            "SELECT name, toString(total_rows) AS total_rows FROM system.tables \
             WHERE database='serving' AND name NOT LIKE '%\\_baru' ORDER BY name",
            None,
        )
        .await
    {
        Ok(r) => r,
        Err(err) => return json!({ "error": err.to_string() }),
    };
    let mut out = Vec::with_capacity(marts.len());
    for m in &marts {
        let name = m.get("name").and_then(Value::as_str).unwrap_or("");
        let cols = match ch
            .rows(
                &format!("SELECT name, type FROM system.columns WHERE database='serving' AND table='{name}' ORDER BY position"),
                None,
            )
            .await
        {
            Ok(r) => r,
            Err(err) => return json!({ "error": err.to_string() }),
        };
        let dimensions: Vec<&str> = cols
            .iter()
            .filter(|c| !is_numeric_type(c.get("type").and_then(Value::as_str).unwrap_or("")))
            .filter_map(|c| c.get("name").and_then(Value::as_str))
            .collect();
        let measures: Vec<&str> = cols
            .iter()
            .filter(|c| is_numeric_type(c.get("type").and_then(Value::as_str).unwrap_or("")))
            .filter_map(|c| c.get("name").and_then(Value::as_str))
            .collect();
        let rows_n: i64 = m
            .get("total_rows")
            .and_then(Value::as_str)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        out.push(
            json!({ "mart": name, "rows": rows_n, "dimensions": dimensions, "measures": measures }),
        );
    }
    json!({ "marts": out })
}

pub(super) async fn list_charts(ch: &ChClient) -> Value {
    match store::list_stored_charts(ch).await {
        Ok(charts) => {
            let out: Vec<Value> = charts
                .iter()
                .map(|c| {
                    json!({
                        "id": c.spec.id, "title": c.spec.title, "kind": c.spec.kind,
                        "mart": c.spec.mart, "source": c.source,
                    })
                })
                .collect();
            json!({ "total": out.len(), "charts": out })
        }
        Err(err) => json!({ "error": err.to_string() }),
    }
}

pub(super) async fn delete_chart(ch: &ChClient, args: &Map<String, Value>) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id wajib" });
    }
    match store::delete_chart(ch, &id).await {
        Ok(()) => json!({ "deleted": true, "id": id }),
        Err(err) => json!({ "error": err.to_string() }),
    }
}
