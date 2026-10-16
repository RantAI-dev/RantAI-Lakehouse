//! Dashboard-board tools (`AI-1`, `AI-2`, `AI-20`): `rename_board`,
//! `delete_board`, `arrange_board`, `move_chart`, `set_board_view_filters`
//! and `save_board_filters`.
//!
//! Five of the six route through the same console route handlers
//! `calc_fields::run` calls (`routes::dashboard::{boards_update,
//! boards_delete}`) so the assistant gets every check the
//! console already runs — `board_edit_allowed`, `validate_filters`, the
//! folder existence check, the Main-board guard, the last-write-wins on
//! layout, the chart-cascade delete — and so a guard added to the
//! console route is automatically applied to the tool too.
//!
//! `set_board_view_filters` is the exception and saves nothing by design
//! (`AI-2`), so it has no handler to call.
//!
//! One seam is not that: `move_chart` saves the chart itself through
//! `store::spec_from_input` + `store::insert_chart`, not through
//! `routes::dashboard::specs_update`. Plan 20 has it save on the new
//! board *before* removing the tile from the old one, so a failure
//! leaves the chart readable rather than on two boards, and that
//! ordering needs the two writes split. `spec_from_input` still runs
//! its own smoke test, so the chart is validated — but a guard added to
//! the `specs_update` handler later would not reach this path, and that
//! is the one place to look when it changes.
//!
//! Plan 20 assigns two new rules to the tool (not the handler), and
//! because they are tool-specific they live here:
//!
//! - `arrange_board` checks each box is on the 12-column grid and is for
//!   a chart that is currently on that board (plan 20 Decisions, lines
//!   74-77). The console's drag UI cannot produce an out-of-grid box or
//!   an off-board chart, a model can. The handler `boards_update` is
//!   still the save path; the `board_edit_allowed` /
//!   folder-`ensure_folder_exists` checks inside it are unchanged.
//! - `move_chart` removes the chart's tile from the old board's layout,
//!   which `specs_update` does not do (plan 20 Decisions, lines 79-80:
//!   "Read first whether `spec_from_input` or `insert_chart` already
//!   does the latter; do it once"). They do not, so the tool does it.

use axum::body::Bytes;
use axum::extract::{Query, State};
use lakehouse_auth::Principal;
use lakehouse_bi::filters::{FilterDef, validate_filters, without_inert};
use lakehouse_bi::store::{self, LayoutMap, TileBox};
use serde_json::{Map, Value, json};

use super::api_result_to_value;
use crate::routes::dashboard::{self, IdQuery};
use crate::state::AppState;

/// The grid `arrange_board` lays tiles on. Plan 20 Decisions, lines 74-77.
const GRID_COLUMNS: i32 = 12;
/// Smallest width/height a tool may set. Plan 20 Decisions, lines 74-77.
const MIN_TILE_SIDE: i32 = 1;

fn arg_str(args: &Map<String, Value>, key: &str) -> String {
    args.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

fn arg_obj<'a>(args: &'a Map<String, Value>, key: &str) -> Option<&'a Map<String, Value>> {
    args.get(key).and_then(Value::as_object)
}

/// Encode the tool's args as the JSON body the console route expects.
/// `body` is the table/board body shape (`BoardEditBody` /
/// `IdQuery`); the route takes the body as `Bytes` so we go through
/// `serde_json::to_vec` rather than re-parsing arg keys.
fn encode_body(value: &Value) -> Bytes {
    Bytes::from(serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec()))
}

/// `rename_board`: change a board's name. Calls `boards_update` with
/// `{id, name}` so the handler's `board_edit_allowed` refuses the built-in
/// Main board and the `name` is trimmed / required.
pub(super) async fn rename_board(state: &AppState, args: &Map<String, Value>) -> Value {
    let id = arg_str(args, "id");
    let name = arg_str(args, "name");
    if id.is_empty() {
        return json!({ "error": "id is required" });
    }
    if name.is_empty() {
        return json!({ "error": "name is required" });
    }
    api_result_to_value(
        dashboard::boards_update(
            State(state.clone()),
            encode_body(&json!({ "id": id, "name": name })),
        )
        .await,
    )
    .await
}

/// `delete_board`: deletes a board AND its charts. The route's own
/// 400 ("invalid dashboard") covers the empty/`"default"` cases. The
/// gate carries the `WriteHigh` approval before this runs.
pub(super) async fn delete_board(state: &AppState, args: &Map<String, Value>) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id is required" });
    }
    api_result_to_value(
        dashboard::boards_delete(State(state.clone()), Query(IdQuery::new(id))).await,
    )
    .await
}

/// Read the current layout for `board` from `state`. Returns an empty
/// `LayoutMap` for the built-in Main board when it has no row yet
/// (`store::get_board` returns `Ok(None)` until the first layout save,
/// per `store.rs:684-687`); a model arrange on the empty Main board
/// therefore becomes a fresh save, the same as the console.
///
/// The `Err` arm carries the *classified* upstream-error JSON, not
/// the raw `ChError` text, so no host / port / table name ever
/// reaches the model (plan 20 Decisions, line 73: "no upstream text
/// reaches the model"). Callers return the `Value` verbatim; the
/// caller is the boundary that hands a tool result to the chat loop.
async fn read_layout(state: &AppState, board: &str) -> Result<LayoutMap, Value> {
    match store::get_board(&state.clickhouse, board).await {
        Ok(Some(b)) => Ok(b.layout.unwrap_or_default()),
        Ok(None) => Ok(LayoutMap::new()),
        Err(err) => Err(upstream_error_json(&err)),
    }
}

/// `arrange_board`: takes only the tiles that change; refuses off-board
/// charts, out-of-grid boxes, and tiles with `w` or `h` below 1
/// (plan 20 Decisions, lines 74-77); merges into the current layout;
/// saves through `boards_update` with `layout`. Layout is replaced whole
/// — last write wins, as in the console.
pub(super) async fn arrange_board(state: &AppState, args: &Map<String, Value>) -> Value {
    let board = arg_str(args, "board");
    if board.is_empty() {
        return json!({ "error": "board is required" });
    }
    let Some(tiles_obj) = arg_obj(args, "tiles") else {
        return json!({ "error": "tiles is required" });
    };
    if tiles_obj.is_empty() {
        return json!({ "error": "tiles must list at least one tile" });
    }

    // Parse every requested tile. Any malformed box (non-number,
    // missing key, or already below 1) is refused before any read.
    let incoming: Vec<(String, TileBox)> = match parse_arrange_tiles(tiles_obj) {
        Ok(t) => t,
        Err(msg) => return json!({ "error": msg }),
    };

    let mut layout = match read_layout(state, &board).await {
        Ok(l) => l,
        Err(json) => return json,
    };

    // Plan 20 Decisions, lines 74-77: a chart id not on the board is
    // refused; the others (those already in the layout being moved,
    // or stored charts assigned to this board) keep their boxes. A
    // chart id present in the current layout is accepted: the call
    // is changing its box, not its board. A chart id absent from
    // the layout must be a stored chart whose `board` is this one.
    //
    // The stored-charts list is read once for the whole call, not
    // per tile, so an N-tile arrange is one ClickHouse round-trip
    // on top of the layout read.
    let stored_charts = match store::list_stored_charts(&state.clickhouse).await {
        Ok(s) => s,
        Err(err) => return upstream_error_json(&err),
    };
    for (chart_id, _) in &incoming {
        if layout.contains_key(chart_id) {
            continue;
        }
        let on_this_board = stored_charts
            .iter()
            .any(|c| c.spec.id == *chart_id && c.board == board);
        if !on_this_board {
            return json!({
                "error": format!("chart {chart_id} is not on board {board}")
            });
        }
    }

    for (chart_id, box_) in incoming {
        layout.insert(chart_id, box_);
    }

    api_result_to_value(
        dashboard::boards_update(
            State(state.clone()),
            encode_body(&json!({ "id": board, "layout": Value::Object(layout_to_json(&layout)) })),
        )
        .await,
    )
    .await
}

/// Parse one `arrange_board` tile map into validated `(chart_id, box)`
/// pairs, refusing any box outside the grid or with a side below 1
/// (plan 20 Decisions, lines 74-77). The bound checks are CPU-only so
/// they are unit-testable without `ClickHouse`.
fn parse_arrange_tiles(tiles_obj: &Map<String, Value>) -> Result<Vec<(String, TileBox)>, String> {
    let mut incoming: Vec<(String, TileBox)> = Vec::with_capacity(tiles_obj.len());
    for (chart_id, raw) in tiles_obj {
        let Some(map) = raw.as_object() else {
            return Err(format!("tile {chart_id} must be an object with x, y, w, h"));
        };
        let Some(box_) = tile_box_from(map) else {
            return Err(format!("tile {chart_id} needs x, y, w, h as numbers"));
        };
        if box_.w < MIN_TILE_SIDE || box_.h < MIN_TILE_SIDE {
            // Plan 20 Decisions, lines 74-77: a box with w or h below 1
            // is refused. The console drag UI cannot produce one.
            return Err(format!(
                "tile {chart_id}: w and h must be at least {MIN_TILE_SIDE}"
            ));
        }
        if box_.x < 0 || box_.y < 0 || box_.x + box_.w > GRID_COLUMNS {
            // Plan 20 Decisions, lines 74-77: a box outside the
            // 12-column grid is refused.
            return Err(format!(
                "tile {chart_id}: box must fit within the {GRID_COLUMNS}-column grid"
            ));
        }
        incoming.push((chart_id.clone(), box_));
    }
    Ok(incoming)
}

fn tile_box_from(map: &Map<String, Value>) -> Option<TileBox> {
    let n = |k: &str| -> Option<i32> {
        let raw = map.get(k).and_then(Value::as_i64)?;
        i32::try_from(raw).ok()
    };
    Some(TileBox {
        x: n("x")?,
        y: n("y")?,
        w: n("w")?,
        h: n("h")?,
    })
}

fn layout_to_json(layout: &LayoutMap) -> Map<String, Value> {
    let mut out = Map::new();
    for (k, b) in layout {
        out.insert(k.clone(), json!({ "x": b.x, "y": b.y, "w": b.w, "h": b.h }));
    }
    out
}

/// `move_chart`: change a chart's board, then remove its tile from the
/// old board's layout. The chart's other fields are unchanged, so the
/// tool reads the stored chart and writes the same `ChartInput` back
/// with the new `board`. `spec_from_input` then runs the same shape /
/// SQL smoke test `specs_update` runs.
pub(super) async fn move_chart(state: &AppState, args: &Map<String, Value>) -> Value {
    let id = arg_str(args, "id");
    let new_board = arg_str(args, "board");
    if id.is_empty() {
        return json!({ "error": "id is required" });
    }
    if new_board.is_empty() {
        return json!({ "error": "board is required" });
    }

    let ch = &state.clickhouse;
    let stored = match store::list_stored_charts(ch).await {
        Ok(s) => s,
        Err(err) => {
            return json!(upstream_error_json(&err));
        }
    };
    let Some(chart) = stored.iter().find(|c| c.spec.id == id) else {
        return json!({ "error": format!("chart {id} not found") });
    };
    let old_board = chart.board.clone();
    if old_board == new_board {
        return json!({
            "ok": true,
            "id": id,
            "board": new_board,
            "note": "chart already on that board",
        });
    }

    // Save the chart on the new board FIRST, so a failure leaves a
    // chart that is still assigned to the old board with its old
    // tile intact — visible and recoverable — rather than a chart
    // that has silently disappeared from a board it is still
    // assigned to (D6). The old tile is removed below only after
    // the new save has succeeded.
    let mut input = chart.def.clone();
    input.board = Some(new_board.clone());
    let Ok(time) = crate::routes::settings::time_context(state).await else {
        return json!({ "error": "The reporting settings could not be read." });
    };
    // A `match`, not `let...else`: the error arm needs `err` to classify it,
    // and a `let...else` arm cannot see the rejected value.
    let spec = match store::spec_from_input(
        ch,
        &input,
        chart.source,
        chart.created_by.as_deref().unwrap_or("ui"),
        Some(id.clone()),
        &time,
    )
    .await
    {
        Ok(spec) => spec,
        Err(err) => {
            return json!(
                crate::upstream_error::bi(&crate::upstream_error::DATABASE, &err).to_json()
            );
        }
    };
    if let Err(err) = store::insert_chart(ch, &spec).await {
        return json!(upstream_error_json(&err));
    }

    // Plan 20 Decisions, lines 79-80: remove the chart's tile from
    // the old board's layout. `specs_update` does not do this. A
    // missing row (Main before its first layout save, or any board
    // that never stored one) means there is no tile to remove — a
    // visible stale tile on the old board is the recoverable shape
    // if this step fails.
    let old_layout = match read_layout(state, &old_board).await {
        Ok(l) => l,
        Err(json) => return json,
    };
    if old_layout.contains_key(&id) {
        let mut next = old_layout;
        next.shift_remove(&id);
        let saved = api_result_to_value(
            dashboard::boards_update(
                State(state.clone()),
                encode_body(&json!({
                    "id": old_board,
                    "layout": Value::Object(layout_to_json(&next)),
                })),
            )
            .await,
        )
        .await;
        if saved.get("error").is_some() {
            return saved;
        }
    }
    json!({
        "ok": true,
        "id": id,
        "from": old_board,
        "to": new_board,
    })
}

fn upstream_error_json(err: &lakehouse_clickhouse::ChError) -> Value {
    crate::upstream_error::report_ch(&crate::upstream_error::DATABASE, err).to_json()
}

/// `set_board_view_filters`: validate filters and hand them to the page.
/// Writes nothing. The console route does not need to be called: there is
/// no board write here. The "no-op on success" invariant is what makes
/// this `Read` (a viewer can do it; ask mode offers it).
///
/// Runs both `validate_filters` AND `without_inert` before returning
/// (plan 20 Decisions, line 82): a view filter that filters nothing
/// (an `in` with no values, a `between` with neither bound, a text
/// op with no text) is dropped from the payload the page applies, so
/// the user never sees a chip that does nothing.
pub(super) fn set_board_view_filters(args: &Map<String, Value>) -> Value {
    let board = arg_str(args, "board");
    if board.is_empty() {
        return json!({ "error": "board is required" });
    }
    let Some(filters) = args.get("filters").and_then(Value::as_array) else {
        return json!({ "error": "filters is required (use [] to clear)" });
    };
    let parsed: Vec<FilterDef> = match serde_json::from_value(Value::Array(filters.clone())) {
        Ok(f) => f,
        Err(err) => return json!({ "error": format!("filters: {err}") }),
    };
    if let Err(msg) = validate_filters(&parsed) {
        return json!({ "error": msg });
    }
    let cleaned = without_inert(&parsed);
    json!({
        "applyViewFilters": true,
        "board": board,
        "filters": cleaned,
    })
}

/// `save_board_filters`: persist the board's default filters.
///
/// The handler's `validate_filters` (limits: 20 max, op shape), the
/// `without_inert` clean (`BI-18` review round 1), `board_edit_allowed`
/// (Main refuses any change but a layout), and folder existence check
/// all run inside `boards_update` (`routes/dashboard.rs:909` /
/// `:940` / `:983` / `:932` / `:996`). The tool does NOT duplicate
/// them — the principle is to call the route, not re-implement it
/// (plan 20 Decisions, line 71-73). Only the shape decode (`serde`
/// parsing the args into `Vec<FilterDef>`) is the tool's job; an
/// unparseable shape is the only thing the tool reports on its own.
pub(super) async fn save_board_filters(
    state: &AppState,
    _principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let id = arg_str(args, "board");
    if id.is_empty() {
        return json!({ "error": "board is required" });
    }
    let Some(filters) = args.get("filters").and_then(Value::as_array) else {
        return json!({ "error": "filters is required (use [] to clear)" });
    };
    let parsed: Vec<FilterDef> = match serde_json::from_value(Value::Array(filters.clone())) {
        Ok(f) => f,
        Err(err) => return json!({ "error": format!("filters: {err}") }),
    };
    api_result_to_value(
        dashboard::boards_update(
            State(state.clone()),
            encode_body(&json!({ "id": id, "filters": parsed })),
        )
        .await,
    )
    .await
}

/// Dispatch one of the six tools by name (one arm in [`super::run_tool`]).
pub(super) async fn run(
    state: &AppState,
    principal: Option<&Principal>,
    name: &str,
    args: &Map<String, Value>,
) -> Value {
    match name {
        "rename_board" => rename_board(state, args).await,
        "delete_board" => delete_board(state, args).await,
        "arrange_board" => arrange_board(state, args).await,
        "move_chart" => move_chart(state, args).await,
        "set_board_view_filters" => set_board_view_filters(args),
        "save_board_filters" => save_board_filters(state, principal, args).await,
        other => json!({ "error": format!("unknown tool: {other}") }),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use lakehouse_bi::store::TileBox;
    use serde_json::{Map, Value, json};

    fn tile(x: i32, y: i32, w: i32, h: i32) -> Value {
        json!({ "x": x, "y": y, "w": w, "h": h })
    }

    /// `arrange_board`'s CPU-only bounds checks: the off-by-one grid
    /// edge, `w` or `h` below 1, and an out-of-grid box. Plan 20
    /// Decisions, lines 74-77.
    #[test]
    fn parse_arrange_tiles_accepts_a_box_that_fits_the_grid() {
        let mut tiles = Map::new();
        tiles.insert("c-1".to_owned(), tile(0, 0, 6, 3));
        tiles.insert("c-2".to_owned(), tile(6, 0, 6, 3));
        let parsed = parse_arrange_tiles(&tiles).expect("fits");
        assert_eq!(parsed.len(), 2);
        assert_eq!(
            parsed[0].1,
            TileBox {
                x: 0,
                y: 0,
                w: 6,
                h: 3
            }
        );
        assert_eq!(
            parsed[1].1,
            TileBox {
                x: 6,
                y: 0,
                w: 6,
                h: 3
            }
        );
    }

    #[test]
    fn parse_arrange_tiles_refuses_a_box_that_overflows_the_grid() {
        let mut tiles = Map::new();
        // x + w = 13 > 12.
        tiles.insert("c-1".to_owned(), tile(6, 0, 7, 1));
        let err = parse_arrange_tiles(&tiles).expect_err("must refuse");
        assert!(err.contains("12-column grid"), "{err}");
        assert!(err.contains("c-1"), "{err}");
    }

    #[test]
    fn parse_arrange_tiles_refuses_w_or_h_below_one() {
        let mut tiles = Map::new();
        tiles.insert("c-1".to_owned(), tile(0, 0, 0, 1));
        let err = parse_arrange_tiles(&tiles).expect_err("w=0");
        assert!(err.contains("w and h must be at least 1"), "{err}");

        tiles.clear();
        tiles.insert("c-1".to_owned(), tile(0, 0, 1, 0));
        let err = parse_arrange_tiles(&tiles).expect_err("h=0");
        assert!(err.contains("w and h must be at least 1"), "{err}");
    }

    #[test]
    fn parse_arrange_tiles_refuses_negative_x_or_y() {
        let mut tiles = Map::new();
        tiles.insert("c-1".to_owned(), tile(-1, 0, 1, 1));
        let err = parse_arrange_tiles(&tiles).expect_err("negative x");
        assert!(err.contains("12-column grid"), "{err}");
    }

    /// `set_board_view_filters`: an empty list clears the view; the
    /// result is the page-apply payload, the tool never writes.
    #[test]
    fn set_board_view_filters_returns_the_view_payload_for_empty_list() {
        let mut args = Map::new();
        args.insert("board".to_owned(), Value::String("b-view".to_owned()));
        args.insert("filters".to_owned(), Value::Array(Vec::new()));
        let out = set_board_view_filters(&args);
        assert_eq!(out["applyViewFilters"], json!(true));
        assert_eq!(out["board"], json!("b-view"));
        assert_eq!(out["filters"], json!([]));
    }

    /// `set_board_view_filters`: 21 filters is refused by
    /// `validate_filters` (the same check the route uses), so the
    /// tool never reaches `ClickHouse`.
    #[test]
    fn set_board_view_filters_refuses_more_than_max_filters() {
        let mut args = Map::new();
        args.insert("board".to_owned(), Value::String("b-view".to_owned()));
        let many: Vec<Value> = (0..21)
            .map(|i| json!({ "column": format!("c{i}"), "values": ["x"] }))
            .collect();
        args.insert("filters".to_owned(), Value::Array(many));
        let out = set_board_view_filters(&args);
        assert!(
            out["error"]
                .as_str()
                .is_some_and(|s| s.contains("at most 20")),
            "{out}"
        );
    }

    /// `set_board_view_filters`: an invalid op is refused. The op is
    /// a `serde` enum on `FilterDef`, so an unknown value fails the
    /// deserialisation of `Vec<FilterDef>` with a message that names
    /// the allowed ops — that is the user-facing refusal here.
    #[test]
    fn set_board_view_filters_refuses_an_invalid_op() {
        let mut args = Map::new();
        args.insert("board".to_owned(), Value::String("b-view".to_owned()));
        args.insert(
            "filters".to_owned(),
            json!([{ "column": "c", "values": [], "op": "fuzzy_match" }]),
        );
        let out = set_board_view_filters(&args);
        let err = out["error"].as_str().expect("error message");
        assert!(
            err.contains("fuzzy_match") || err.contains("filters:"),
            "expected the unknown op to be named in the refusal, got: {err}"
        );
    }

    // `save_board_filters` does NOT get a 21-filters test here, and
    // deliberately so. The limit lives in the route
    // (`routes/dashboard.rs:942`, `validate_filters`), the tool does not
    // duplicate it, and this test would have to call `save_board_filters`
    // — which needs an `&AppState`, so a live `ClickHouse` that this
    // harness does not have. A test that calls
    // `lakehouse_bi::filters::validate_filters` instead would only
    // re-assert the library against itself and stay green if the route's
    // call were deleted, so it is not worth having. The limit is covered
    // where it lives, by `lakehouse_bi::filters`' own tests; the refusal
    // *message* is covered through a real call path by
    // `set_board_view_filters_refuses_more_than_max_filters` above.
}
