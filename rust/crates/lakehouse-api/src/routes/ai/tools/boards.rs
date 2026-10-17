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
//! Merge of main into phase 3 (`BI-18` part C): every handler takes the
//! signed-in person (`Extension<Principal>`) and every read is as that
//! person (`Viewer`), so a dashboard in somebody else's personal space or in
//! the trash is "not found" to these tools as to the console. With nobody
//! attached nothing is changed, as for the tab tools.
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

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Query, State};
use lakehouse_auth::Principal;
use lakehouse_bi::access::Viewer;
use lakehouse_bi::filters::{FilterDef, validate_filters, without_inert};
use lakehouse_bi::store::{self, LayoutMap, TileBox};
use serde_json::{Map, Value, json};

use super::api_result_to_value;
use super::dashboards::refused;
use crate::routes::dashboard::{self, IdQuery};
use crate::routes::dashboard_organise::{
    drop_tile, place_new_chart, require_board, require_chart_board, require_editable_chart,
    snapshot, viewer_of,
};
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
    // `serde_json::to_vec` cannot fail on a `Value`: the value is already
    // JSON, so there is nothing left that can fail to represent it. The
    // fallback is unreachable rather than a live path, and `expect` is
    // denied outside `#[cfg(test)]`, so an empty body is what stands in for
    // "cannot happen" here.
    Bytes::from(serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec()))
}

/// The error for a call with no signed-in person attached. The console
/// handlers these tools call act as the caller (the personal space, the
/// trash, the version author), so with nobody attached there is nobody to act
/// as and nothing is changed.
fn no_caller() -> Value {
    json!({ "error": "the board tools change a dashboard as the signed-in person, and there is none." })
}

/// `rename_board`: change a board's name. Calls `boards_update` with
/// `{id, name}` so the handler's `board_edit_allowed` refuses the built-in
/// Main board and the `name` is trimmed / required.
pub(super) async fn rename_board(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let Some(principal) = principal else {
        return no_caller();
    };
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
            Extension(principal.clone()),
            encode_body(&json!({ "id": id, "name": name })),
        )
        .await,
    )
    .await
}

/// `delete_board`: moves a board AND its charts to the trash (restorable for
/// 30 days, `BI-18` part C), exactly as the console's delete does: the same
/// handler, which requires a board the caller may see. The route's own
/// 400 ("invalid dashboard") covers the empty/`"default"` cases. The
/// gate carries the `WriteHigh` approval before this runs.
pub(super) async fn delete_board(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let Some(principal) = principal else {
        return no_caller();
    };
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id is required" });
    }
    api_result_to_value(
        dashboard::boards_delete(
            State(state.clone()),
            Extension(principal.clone()),
            Query(IdQuery::new(id)),
        )
        .await,
    )
    .await
}

/// Read the current layout for `board` from `state`. Returns an empty
/// `LayoutMap` for the built-in Main board when it has no row yet
/// (`store::get_board` returns `Ok(None)` until the first layout save,
/// per `store.rs:904`); a model arrange on the empty Main board
/// therefore becomes a fresh save, the same as the console.
///
/// The `Err` arm carries the *classified* upstream-error JSON, not
/// the raw `ChError` text, so no host / port / table name ever
/// reaches the model (plan 20 Decisions, line 73: "no upstream text
/// reaches the model"). Callers return the `Value` verbatim; the
/// caller is the boundary that hands a tool result to the chat loop.
async fn read_layout(state: &AppState, board: &str, viewer: &Viewer) -> Result<LayoutMap, Value> {
    match store::get_board(&state.clickhouse, board, viewer).await {
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
pub(super) async fn arrange_board(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let Some(principal) = principal else {
        return no_caller();
    };
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

    // Merge of main into phase 3: a dashboard the caller cannot see (in the
    // trash, in somebody else's personal space, unknown) is "not found" before
    // anything of it is read.
    let viewer = viewer_of(principal);
    if board != store::DEFAULT_BOARD_ID
        && let Err(gate) = require_board(&state.clickhouse, &board, &viewer).await
    {
        return refused(gate);
    }
    let mut layout = match read_layout(state, &board, &viewer).await {
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
    let stored_charts = match store::list_stored_charts(&state.clickhouse, &viewer).await {
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

    for (chart_id, mut box_) in incoming {
        // The tool moves and resizes a tile; it does not change its tab, so a
        // tile already in the layout keeps the tab it names (a tile new to
        // the layout names none, and the board save stamps it, BI-18 part C
        // review fix R1).
        box_.tab = layout.get(&chart_id).and_then(|old| old.tab.clone());
        layout.insert(chart_id, box_);
    }

    api_result_to_value(
        dashboard::boards_update(
            State(state.clone()),
            Extension(principal.clone()),
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
        if box_.x < 0
            || box_.y < 0
            // `box_.x + box_.w` is `i32 + i32` and overflows at `i32::MAX`
            // (panicking in debug, wrapping to a negative in release, where
            // the negative then fails `> GRID_COLUMNS` and the off-grid box
            // is silently accepted). Both operands are `i32`, so the sum
            // always fits in `i64` and the comparison is exact.
            || i64::from(box_.x) + i64::from(box_.w) > i64::from(GRID_COLUMNS)
        {
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
        tab: None,
    })
}

fn layout_to_json(layout: &LayoutMap) -> Map<String, Value> {
    // The tile's own serialisation, so the tab it names (when it names one)
    // rides along; a hand-built `{x, y, w, h}` would drop it.
    let mut out = Map::new();
    for (k, b) in layout {
        out.insert(k.clone(), serde_json::to_value(b).unwrap_or(Value::Null));
    }
    out
}

/// `move_chart`: change a chart's board, take its tile off the old board and
/// give it one on the new board, like a chart created there. The chart's
/// other fields are unchanged, so the tool reads the stored chart and writes
/// the same `ChartInput` back with the new `board`. `spec_from_input` then
/// runs the same shape / SQL smoke test `specs_update` runs.
///
/// Merge of main into phase 3 (`BI-18` part C): the chart and both boards are
/// read as the caller; the old board's tile goes through `drop_tile` (the
/// layout save that closes the space it leaves, review fix R4/R9, and stamps
/// the other tiles' tabs, R1); the new board's through `place_new_chart` (a
/// tile on its first tab when it has tabs, R1); and each board that changed
/// records a version, as the console's chart move does.
pub(super) async fn move_chart(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let Some(principal) = principal else {
        return no_caller();
    };
    let id = arg_str(args, "id");
    let new_board = arg_str(args, "board");
    if id.is_empty() {
        return json!({ "error": "id is required" });
    }
    if new_board.is_empty() {
        return json!({ "error": "board is required" });
    }

    let ch = &state.clickhouse;
    let viewer = viewer_of(principal);
    // A chart in the trash or on a board the caller cannot see, and a target
    // in the trash or in somebody else's personal space, are "not found".
    if let Err(gate) = require_editable_chart(ch, &id, &viewer).await {
        return refused(gate);
    }
    if let Err(gate) = require_chart_board(ch, Some(&new_board), &viewer).await {
        return refused(gate);
    }
    let stored = match store::list_stored_charts(ch, &viewer).await {
        Ok(s) => s,
        Err(err) => {
            return upstream_error_json(&err);
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

    // Plan 20 Decisions, lines 79-80: remove the chart's tile from the old
    // board's layout. `specs_update` does not do this. A board with no tile
    // for it is left as it is; a failure here is logged and swallowed (the
    // chart has moved), and leaves a stale tile on the old board, which is
    // the recoverable shape.
    drop_tile(ch, &old_board, &id).await;
    place_new_chart(ch, &new_board, &id).await;
    snapshot(ch, &old_board, principal).await;
    snapshot(ch, &new_board, principal).await;
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
/// all run inside `boards_update` (`routes/dashboard.rs:1145` /
/// `:1155` / `:1163` / `:1204` / `:1217`). The tool does NOT duplicate
/// them — the principle is to call the route, not re-implement it
/// (plan 20 Decisions, line 71-73). Only the shape decode (`serde`
/// parsing the args into `Vec<FilterDef>`) is the tool's job; an
/// unparseable shape is the only thing the tool reports on its own.
pub(super) async fn save_board_filters(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let Some(principal) = principal else {
        return no_caller();
    };
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
            Extension(principal.clone()),
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
        "rename_board" => rename_board(state, principal, args).await,
        "delete_board" => delete_board(state, principal, args).await,
        "arrange_board" => arrange_board(state, principal, args).await,
        "move_chart" => move_chart(state, principal, args).await,
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
                h: 3,
                tab: None
            }
        );
        assert_eq!(
            parsed[1].1,
            TileBox {
                x: 6,
                y: 0,
                w: 6,
                h: 3,
                tab: None
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

    /// `tile_box_from` parses every component as an `i32`, so `x` and
    /// `w` can both reach `i32::MAX`. The horizontal-extent check must
    /// refuse such a box (plan 20 Decisions, lines 74-77) — an `i32`
    /// sum overflows in debug (panicking in the request handler) and
    /// wraps in release to a large negative, where the negative then
    /// fails `> GRID_COLUMNS` and the off-grid box is silently
    /// accepted. The same wire input has to be refused, not silently
    /// saved.
    #[test]
    fn parse_arrange_tiles_refuses_x_at_i32_max() {
        let mut tiles = Map::new();
        tiles.insert("c-overflow".to_owned(), tile(i32::MAX, 0, 1, 1));
        let err = parse_arrange_tiles(&tiles).expect_err("must refuse");
        assert!(err.contains("12-column grid"), "{err}");
        assert!(err.contains("c-overflow"), "{err}");
    }

    /// The overflow is in the SUM, not in `x` alone: `x` and `w` are each
    /// individually representable, and `(i32::MAX - 5) + 10` is what wraps.
    /// A fix that only range-checked `x` would let this box through.
    #[test]
    fn parse_arrange_tiles_refuses_a_box_whose_sum_overflows_i32() {
        let mut tiles = Map::new();
        tiles.insert("c-sum-overflow".to_owned(), tile(i32::MAX - 5, 0, 10, 1));
        let err = parse_arrange_tiles(&tiles).expect_err("must refuse");
        assert!(err.contains("12-column grid"), "{err}");
        assert!(err.contains("c-sum-overflow"), "{err}");
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
            // Every parse error is prefixed `filters: `, so asserting on that
            // cannot fail; what matters is that the refusal names the op the
            // model actually sent, which is the user-facing part.
            err.contains("fuzzy_match"),
            "expected the unknown op to be named in the refusal, got: {err}"
        );
    }

    // ── Merge of main into phase 3: the board tools keep phase 3's rules ──
    //
    // A wiremock stands in for `ClickHouse`: rules are matched in the order
    // they are mounted, so the narrow ones come first.

    use std::collections::HashMap;

    use lakehouse_auth::{PermissionSet, PrincipalId};
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::config::Config;

    fn person() -> Principal {
        Principal {
            id: PrincipalId::User(uuid::Uuid::from_u128(7)),
            tenant_ids: Vec::new(),
            display_name: "Rina".to_owned(),
            permissions: PermissionSet::parse("dashboard:write"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    fn args_of(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap_or_default()
    }

    fn board_row(id: &str, layout: &str, tabs: &str) -> Value {
        json!({
            "id": id, "name": id, "description": "", "created_by": "t",
            "layout_json": layout, "tabs_json": tabs,
            "filters_json": "[]", "created_at": "2026-10-14 00:00:00",
        })
    }

    fn chart_row(id: &str, board: &str) -> Value {
        let spec = json!({ "id": id, "title": id, "kind": "bar", "mart": "m", "sql": "SELECT 1", "x": "d", "y": "v" });
        let def = json!({ "title": id, "kind": "text", "text": "hello" });
        json!({
            "id": id, "board": board, "created_by": "ui", "created_at": "2026-01-01 00:00:00",
            "spec_json": json!({ "spec": spec, "def": def }).to_string(),
        })
    }

    async fn mount(server: &MockServer, needle: &str, data: Vec<Value>) {
        Mock::given(method("POST"))
            .and(body_string_contains(needle))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "meta": [], "data": data, "rows": data.len() })),
            )
            .mount(server)
            .await;
    }

    /// Boards `b1` (tabs A and B; `c1`, `c2` name none, `c3` is on B; charts
    /// `c1` to `c3`) and `b2` (one tab `t_x`, empty), both live and the
    /// caller's to see; every other statement answers with no rows. With
    /// `private_to_other`, `b1` belongs to somebody else's personal space.
    async fn state_on(server: &MockServer, private_to_other: bool) -> AppState {
        let owner = if private_to_other {
            "00000000-0000-0000-0000-0000000000ff"
        } else {
            ""
        };
        mount(
            server,
            "SELECT is_deleted, owner_id FROM console.bi_board FINAL WHERE id = 'b1'",
            vec![json!({ "is_deleted": 0, "owner_id": owner })],
        )
        .await;
        mount(
            server,
            "SELECT is_deleted, owner_id FROM console.bi_board",
            vec![json!({ "is_deleted": 0, "owner_id": "" })],
        )
        .await;
        mount(
            server,
            "SELECT is_deleted, board FROM console.bi_chart",
            vec![json!({ "is_deleted": 0, "board": "b1" })],
        )
        .await;
        let b1 = board_row(
            "b1",
            r#"{"c1":{"x":0,"y":0,"w":6,"h":4},"c2":{"x":0,"y":4,"w":6,"h":4},"c3":{"x":6,"y":0,"w":6,"h":4,"tab":"t_b"}}"#,
            r#"[{"id":"t_a","title":"A"},{"id":"t_b","title":"B"}]"#,
        );
        let b2 = board_row("b2", "{}", r#"[{"id":"t_x","title":"X"}]"#);
        mount(server, "WHERE is_deleted = 0 AND id='b1'", vec![b1]).await;
        mount(server, "WHERE is_deleted = 0 AND id='b2'", vec![b2]).await;
        mount(
            server,
            "FROM console.bi_chart FINAL",
            vec![
                chart_row("c1", "b1"),
                chart_row("c2", "b1"),
                chart_row("c3", "b1"),
            ],
        )
        .await;
        mount(server, "", Vec::new()).await;
        let env = HashMap::from([
            ("CH_URL".to_owned(), server.uri()),
            (
                "DATABASE_URL".to_owned(),
                "not a connection string".to_owned(),
            ),
        ]);
        AppState::new(Config::from_map(&env).expect("a valid test config"))
    }

    async fn sent(server: &MockServer) -> Vec<String> {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .collect()
    }

    fn last_with<'a>(sent: &'a [String], needle: &str) -> &'a str {
        sent.iter()
            .rfind(|s| s.contains(needle))
            .unwrap_or_else(|| panic!("no statement with {needle:?} in {sent:#?}"))
    }

    fn writes(sent: &[String]) -> usize {
        sent.iter()
            .filter(|s| s.trim_start().starts_with("INSERT"))
            .count()
    }

    fn version_inserts(sent: &[String], board: &str) -> usize {
        sent.iter()
            .filter(|s| {
                s.contains("INSERT INTO console.bi_board_version")
                    && s.contains(&format!("VALUES ('{board}'"))
            })
            .count()
    }

    /// The tools change a dashboard as the signed-in person; with nobody
    /// attached nothing is read or written.
    #[tokio::test]
    async fn the_board_tools_do_nothing_without_a_person_to_act_as() {
        let server = MockServer::start().await;
        let state = state_on(&server, false).await;
        let calls = [
            ("rename_board", json!({ "id": "b1", "name": "N" })),
            ("delete_board", json!({ "id": "b1" })),
            (
                "arrange_board",
                json!({ "board": "b1", "tiles": { "c1": tile(0, 0, 3, 2) } }),
            ),
            ("move_chart", json!({ "id": "c1", "board": "b2" })),
            (
                "save_board_filters",
                json!({ "board": "b1", "filters": [] }),
            ),
        ];
        for (name, args) in calls {
            let reply = run(&state, None, name, &args_of(args)).await;
            assert!(
                reply["error"]
                    .as_str()
                    .is_some_and(|e| e.contains("signed-in")),
                "{name}: {reply}"
            );
        }
        assert!(sent(&server).await.is_empty());
    }

    /// `delete_board` moves the board and its charts to the trash exactly as
    /// the console's delete does: `is_deleted = 2` with `deleted_with`, never
    /// the old tombstone.
    #[tokio::test]
    async fn delete_board_moves_the_board_and_its_charts_to_the_trash() {
        let server = MockServer::start().await;
        let state = state_on(&server, false).await;
        let reply = run(
            &state,
            Some(&person()),
            "delete_board",
            &args_of(json!({ "id": "b1" })),
        )
        .await;
        assert_eq!(reply["ok"], json!(true), "{reply}");
        let sent = sent(&server).await;
        let charts = last_with(&sent, "INSERT INTO console.bi_chart");
        assert!(charts.contains(", 2, now(), 'Rina', 'b1'"), "{charts}");
        let board = last_with(&sent, "INSERT INTO console.bi_board (");
        assert!(board.contains(", 2, now(), 'Rina'"), "{board}");
        assert_eq!(writes(&sent), 2, "only the two trash statements: {sent:#?}");
    }

    /// `arrange_board` keeps the tab each tile names, stamps the tiles that
    /// name none with the board's first tab (R1) and records a version.
    #[tokio::test]
    async fn arrange_board_keeps_the_tabs_and_records_a_version() {
        let server = MockServer::start().await;
        let state = state_on(&server, false).await;
        let reply = run(
            &state,
            Some(&person()),
            "arrange_board",
            &args_of(json!({ "board": "b1", "tiles": { "c3": tile(0, 0, 12, 2) } })),
        )
        .await;
        assert_eq!(reply["ok"], json!(true), "{reply}");
        let sent = sent(&server).await;
        let saved = last_with(&sent, "INSERT INTO console.bi_board (");
        assert!(
            saved.contains(r#""c3":{"x":0,"y":0,"w":12,"h":2,"tab":"t_b"}"#),
            "{saved}"
        );
        assert!(
            saved.contains(r#""c1":{"x":0,"y":0,"w":6,"h":4,"tab":"t_a"}"#),
            "{saved}"
        );
        assert_eq!(version_inserts(&sent, "b1"), 1, "{sent:#?}");
    }

    /// `save_board_filters` runs the console's tab rules: a filter tied to a
    /// tab the board does not have is refused and nothing is written; a tie
    /// to a real tab is saved and recorded as a version.
    #[tokio::test]
    async fn save_board_filters_checks_the_ties_to_tabs() {
        let server = MockServer::start().await;
        let state = state_on(&server, false).await;
        let tied = |tab: &str| json!({ "board": "b1", "filters": [{ "column": "p", "values": ["x"], "tabs": [tab] }] });
        let reply = run(
            &state,
            Some(&person()),
            "save_board_filters",
            &args_of(tied("t_nope")),
        )
        .await;
        assert!(reply.get("error").is_some(), "{reply}");
        assert_eq!(writes(&sent(&server).await), 0);

        let reply = run(
            &state,
            Some(&person()),
            "save_board_filters",
            &args_of(tied("t_b")),
        )
        .await;
        assert_eq!(reply["ok"], json!(true), "{reply}");
        let sent = sent(&server).await;
        assert_eq!(version_inserts(&sent, "b1"), 1, "{sent:#?}");
    }

    /// `rename_board` records a version, as the console's rename does.
    #[tokio::test]
    async fn rename_board_records_a_version() {
        let server = MockServer::start().await;
        let state = state_on(&server, false).await;
        let reply = run(
            &state,
            Some(&person()),
            "rename_board",
            &args_of(json!({ "id": "b1", "name": "Renamed" })),
        )
        .await;
        assert_eq!(reply["ok"], json!(true), "{reply}");
        assert_eq!(version_inserts(&sent(&server).await, "b1"), 1);
    }

    /// `move_chart` puts the chart on the target board, takes its tile off the
    /// source and closes the space it leaves (R4/R9), gives it a tile on the
    /// target's first tab (R1), and records a version on both boards.
    #[tokio::test]
    async fn move_chart_closes_the_space_and_places_the_tile_on_the_first_tab() {
        let server = MockServer::start().await;
        let state = state_on(&server, false).await;
        let reply = run(
            &state,
            Some(&person()),
            "move_chart",
            &args_of(json!({ "id": "c1", "board": "b2" })),
        )
        .await;
        assert_eq!(reply["ok"], json!(true), "{reply}");
        let sent = sent(&server).await;
        assert!(
            sent.iter()
                .any(|s| s.contains("INSERT INTO console.bi_chart") && s.contains("'b2'")),
            "{sent:#?}"
        );
        let source = sent
            .iter()
            .rfind(|s| s.contains("INSERT INTO console.bi_board (") && s.contains("'b1'"))
            .expect("the source board was saved");
        assert!(!source.contains(r#""c1""#), "{source}");
        // c2 rose into the space c1 left, and is stamped with its tab.
        assert!(
            source.contains(r#""c2":{"x":0,"y":0,"w":6,"h":4,"tab":"t_a"}"#),
            "{source}"
        );
        let target = sent
            .iter()
            .rfind(|s| s.contains("INSERT INTO console.bi_board (") && s.contains("'b2'"))
            .expect("the target board was saved");
        assert!(
            target.contains(r#""c1":"#) && target.contains(r#""tab":"t_x""#),
            "{target}"
        );
        assert_eq!(version_inserts(&sent, "b1"), 1, "{sent:#?}");
        assert_eq!(version_inserts(&sent, "b2"), 1, "{sent:#?}");
    }

    /// A dashboard in somebody else's personal space is "not found" to every
    /// board tool, as to the console, and nothing is written.
    #[tokio::test]
    async fn a_board_in_another_persons_space_is_not_found_to_the_board_tools() {
        let calls = [
            ("rename_board", json!({ "id": "b1", "name": "N" })),
            ("delete_board", json!({ "id": "b1" })),
            (
                "arrange_board",
                json!({ "board": "b1", "tiles": { "c1": tile(0, 0, 3, 2) } }),
            ),
            ("move_chart", json!({ "id": "c1", "board": "b2" })),
            (
                "save_board_filters",
                json!({ "board": "b1", "filters": [] }),
            ),
        ];
        for (name, args) in calls {
            let server = MockServer::start().await;
            let state = state_on(&server, true).await;
            let reply = run(&state, Some(&person()), name, &args_of(args)).await;
            let error = reply["error"].as_str().unwrap_or_default().to_lowercase();
            assert!(error.contains("not found"), "{name}: {reply}");
            let sent = sent(&server).await;
            assert_eq!(writes(&sent), 0, "{name}: {sent:#?}");
        }
    }

    // `save_board_filters`' own 21-filter and invalid-op refusals are NOT
    // tested in this module, because they cannot be: the tool takes an
    // `&AppState`, and this unit-test module has none to give it. They are not
    // untested, though — both run before any `ClickHouse` call (the invalid op
    // in the tool's own parse, the count in the route's `validate_filters`
    // ahead of its first store call), so they are asserted through the HTTP
    // harness in `tests/ai_tool.rs` as
    // `save_board_filters_refuses_more_than_twenty_filters` and
    // `save_board_filters_refuses_an_invalid_op`.
    //
    // What is genuinely untested here is the half no harness can reach:
    // `save_board_filters` writing a valid filter list to `bi_board.filters_json`
    // needs a real `ClickHouse`. That gap is recorded in `tests/ai_tool.rs`.
}
