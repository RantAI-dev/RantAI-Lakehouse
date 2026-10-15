//! Tab tools (`BI-18` part C, with `AI-9`): `create_tab`, `rename_tab`,
//! `move_charts_to_tab`, `move_tab` and `delete_tab`.
//!
//! # Posture
//!
//! Each tool works out the new tabs, layout and filters with the same pure
//! functions the console's rules are written in (`lakehouse_bi::layout`) and
//! then saves them through the console route's own handler
//! (`routes::dashboard::boards_update`), so the assistant cannot reach what
//! the console cannot: the same checks run before the first write (a tile on a
//! tab that does not exist, a filter tied to one, a title that is too long),
//! and the same permission, `dashboard:write`, is on each registry entry and
//! enforced by the gate. `delete_tab` is `WriteHigh` and asks for approval like
//! `delete_chart`.
//!
//! The built-in dashboard is one page and has no tabs: the tools refuse it and
//! say to duplicate it. A tab is named by its id or its (unambiguous) title.
//! The database's text never reaches the model: a failed save is the route's
//! own fixed sentence.

use axum::Extension;
use axum::body::Bytes;
use axum::extract::State;
use lakehouse_auth::Principal;
use lakehouse_bi::layout::{self, Tab};
use lakehouse_bi::store::{self, Board};
use serde_json::{Map, Value, json};

use super::{api_result_to_value, arg_str};
use crate::routes::dashboard;
use crate::routes::dashboard_organise::viewer_of;
use crate::state::AppState;
use crate::upstream_error;

/// Whether `name` is one of the tab tools (one guard arm in `run_tool`).
pub(super) fn handles(name: &str) -> bool {
    matches!(
        name,
        "create_tab" | "rename_tab" | "move_charts_to_tab" | "move_tab" | "delete_tab"
    )
}

/// Dispatch one of the five tools by name.
pub(super) async fn run(
    state: &AppState,
    principal: Option<&Principal>,
    name: &str,
    args: &Map<String, Value>,
) -> Value {
    // BI-18 part C, part 2: the save below is the console route's own handler,
    // which reads the board as the caller (the personal space, the trash). With
    // nobody attached there is no caller to act as, so nothing is changed.
    let Some(principal) = principal else {
        return json!({ "error": "the tab tools change a dashboard as the signed-in person, and there is none." });
    };
    let board = match load_board(state, principal, args).await {
        Ok(board) => board,
        Err(err) => return err,
    };
    let who = (state, principal);
    match name {
        "create_tab" => create_tab(who, board, args).await,
        "rename_tab" => rename_tab(who, board, args).await,
        "move_charts_to_tab" => move_charts_to_tab(who, board, args).await,
        "move_tab" => move_tab(who, board, args).await,
        "delete_tab" => delete_tab(who, board, args).await,
        other => json!({ "error": format!("unknown tool: {other}") }),
    }
}

/// The application state and the person the tool acts as.
type Who<'a> = (&'a AppState, &'a Principal);

/// The board the call names, or the error to hand back.
async fn load_board(
    state: &AppState,
    principal: &Principal,
    args: &Map<String, Value>,
) -> Result<Board, Value> {
    let id = arg_str(args, "board");
    if id.is_empty() {
        return Err(json!({ "error": "board is required (an id from list_boards)." }));
    }
    if id == store::DEFAULT_BOARD_ID {
        return Err(json!({
            "error": "the built-in dashboard has no tabs; duplicate it first, or use another dashboard."
        }));
    }
    match store::get_board(&state.clickhouse, &id, &viewer_of(principal)).await {
        Ok(Some(board)) => Ok(board),
        Ok(None) => Err(json!({ "error": "dashboard not found." })),
        Err(err) => Err(upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json()),
    }
}

/// Save through the console route and fold its answer into the tool's.
async fn save((state, principal): Who<'_>, board: &str, changes: Value, ok: Value) -> Value {
    let mut body = changes;
    body["id"] = json!(board);
    let bytes = Bytes::from(serde_json::to_vec(&body).unwrap_or_default());
    let result = api_result_to_value(
        dashboard::boards_update(State(state.clone()), Extension(principal.clone()), bytes).await,
    )
    .await;
    if result.get("error").is_some() {
        return result;
    }
    ok
}

fn tabs_of(board: &Board) -> Vec<Tab> {
    board.tabs.clone().unwrap_or_default()
}

fn no_such_tab(board: &Board, key: &str) -> Value {
    let known: Vec<&str> = board
        .tabs
        .iter()
        .flatten()
        .map(|t| t.title.as_str())
        .collect();
    if known.is_empty() {
        return json!({ "error": "the dashboard has no tabs; use create_tab first." });
    }
    json!({
        "error": format!(
            "no tab named {key:?} (or two share that title); the tabs are: {}.",
            known.join(", ")
        )
    })
}

async fn create_tab(who: Who<'_>, board: Board, args: &Map<String, Value>) -> Value {
    let tabs = tabs_of(&board);
    let layout = board.layout.clone().unwrap_or_default();
    let id = layout::new_tab_id();
    let (next, layout) = match layout::add_tab(&tabs, &layout, &arg_str(args, "title"), &id) {
        Ok(done) => done,
        Err(message) => return json!({ "error": message }),
    };
    let first = tabs.is_empty();
    let title = next.last().map(|t| t.title.clone()).unwrap_or_default();
    let note = if first {
        "This is the dashboard's first tab and it holds every chart already on it. \
         Call create_tab again for the tab to move charts to, then move_charts_to_tab."
    } else {
        "The tab is empty; use move_charts_to_tab to put charts on it."
    };
    let ok = json!({
        "created": true, "board": board.id, "id": id, "title": title,
        "tabs": next, "note": note,
    });
    save(
        who,
        &board.id,
        json!({ "tabs": next, "layout": layout }),
        ok,
    )
    .await
}

async fn rename_tab(who: Who<'_>, board: Board, args: &Map<String, Value>) -> Value {
    let tabs = tabs_of(&board);
    let key = arg_str(args, "tab");
    let Some(tab) = layout::find_tab(&tabs, &key) else {
        return no_such_tab(&board, &key);
    };
    let next = match layout::rename_tab(&tabs, &tab.id, &arg_str(args, "title")) {
        Ok(next) => next,
        Err(message) => return json!({ "error": message }),
    };
    let ok = json!({ "renamed": true, "board": board.id, "id": tab.id, "tabs": next });
    save(who, &board.id, json!({ "tabs": next }), ok).await
}

async fn move_charts_to_tab(who: Who<'_>, board: Board, args: &Map<String, Value>) -> Value {
    let tabs = tabs_of(&board);
    let key = arg_str(args, "tab");
    let Some(tab) = layout::find_tab(&tabs, &key) else {
        return no_such_tab(&board, &key);
    };
    let wanted: Vec<String> = args
        .get("chartIds")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    if wanted.is_empty() {
        return json!({ "error": "chartIds is required: the ids from list_charts." });
    }
    // Only charts that are on this dashboard; a stray id is named, not guessed at.
    let on_board: Vec<String> =
        match store::list_stored_charts(&who.0.clickhouse, &viewer_of(who.1)).await {
            Ok(all) => all
                .into_iter()
                .filter(|c| c.board == board.id)
                .map(|c| c.spec.id)
                .collect(),
            Err(err) => {
                return upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json();
            }
        };
    if let Some(stray) = wanted.iter().find(|id| !on_board.contains(id)) {
        return json!({ "error": format!("chart {stray} is not on this dashboard.") });
    }
    let layout = board.layout.clone().unwrap_or_default();
    let moved = match layout::move_to_tab(&tabs, &layout, &tab.id, &wanted) {
        Ok(moved) => moved,
        Err(message) => return json!({ "error": message }),
    };
    let ok = json!({
        "moved": true, "board": board.id, "tab": tab.id, "title": tab.title, "chartIds": wanted,
    });
    save(who, &board.id, json!({ "layout": moved }), ok).await
}

/// Reorder the tabs: `tabs` lists every tab in the new order, or `tab` goes to
/// `position` (1 is first). Both end in `layout::reorder_tabs`, the check
/// `PUT /api/dashboard/boards` runs again on save.
async fn move_tab(who: Who<'_>, board: Board, args: &Map<String, Value>) -> Value {
    let tabs = tabs_of(&board);
    let listed: Vec<String> = args
        .get("tabs")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let next = if listed.is_empty() {
        let key = arg_str(args, "tab");
        let Some(tab) = layout::find_tab(&tabs, &key) else {
            return no_such_tab(&board, &key);
        };
        let Some(position) = args
            .get("position")
            .and_then(Value::as_u64)
            .and_then(|p| usize::try_from(p).ok())
        else {
            return json!({ "error": "give either tabs (every tab, in the new order) or tab with position (1 is first)." });
        };
        layout::move_tab(&tabs, &tab.id, position)
    } else {
        let mut order = Vec::with_capacity(listed.len());
        for key in &listed {
            let Some(tab) = layout::find_tab(&tabs, key) else {
                return no_such_tab(&board, key);
            };
            order.push(tab.id.clone());
        }
        layout::reorder_tabs(&tabs, &order)
    };
    let next = match next {
        Ok(next) => next,
        Err(message) => return json!({ "error": message }),
    };
    let ok = json!({ "moved": true, "board": board.id, "tabs": next });
    save(who, &board.id, json!({ "tabs": next }), ok).await
}

async fn delete_tab(who: Who<'_>, board: Board, args: &Map<String, Value>) -> Value {
    let tabs = tabs_of(&board);
    let key = arg_str(args, "tab");
    let Some(tab) = layout::find_tab(&tabs, &key) else {
        return no_such_tab(&board, &key);
    };
    let layout = board.layout.clone().unwrap_or_default();
    let filters = board.filters.clone().unwrap_or_default();
    let (left, moved, filters_after) = match layout::delete_tab(&tabs, &layout, &filters, &tab.id) {
        Ok(done) => done,
        Err(message) => return json!({ "error": message }),
    };
    let mut changes = json!({ "tabs": left, "layout": moved });
    // The filters are sent only when a tie changed, as the console does.
    if filters_after != filters {
        changes["filters"] = json!(filters_after);
    }
    let ok = json!({
        "deleted": true, "board": board.id, "id": tab.id, "title": tab.title,
        "note": "Its charts moved to the first remaining tab.",
    });
    save(who, &board.id, changes, ok).await
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use lakehouse_auth::{PermissionSet, PrincipalId};
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::config::Config;

    async fn state_on(server: &MockServer) -> AppState {
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "meta": [], "data": [], "rows": 0 })),
            )
            .mount(server)
            .await;
        let env = HashMap::from([("CH_URL".to_owned(), server.uri())]);
        AppState::new(Config::from_map(&env).expect("a valid test config"))
    }

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

    fn args() -> Map<String, Value> {
        let mut a = Map::new();
        a.insert("board".to_owned(), json!("b_draft"));
        a.insert("title".to_owned(), json!("Maps"));
        a
    }

    /// The save is the console route's own handler, which needs the person it
    /// acts as: with nobody attached nothing is read or written.
    #[tokio::test]
    async fn the_tab_tools_do_nothing_without_a_person_to_act_as() {
        let server = MockServer::start().await;
        let state = state_on(&server).await;
        let reply = run(&state, None, "create_tab", &args()).await;
        assert!(
            reply["error"].as_str().unwrap().contains("signed-in"),
            "{reply}"
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    /// A dashboard the person cannot see (private to somebody else, or in the
    /// trash) is read with the owner restriction and reported as not found;
    /// nothing is written.
    #[tokio::test]
    async fn the_tab_tools_find_only_dashboards_the_person_may_see() {
        let server = MockServer::start().await;
        let state = state_on(&server).await;
        let reply = run(&state, Some(&person()), "create_tab", &args()).await;
        assert_eq!(reply["error"], json!("dashboard not found."));
        let sent: Vec<String> = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .collect();
        let read = sent
            .iter()
            .find(|s| s.contains("FROM console.bi_board FINAL WHERE is_deleted = 0"))
            .expect("the board was read");
        assert!(
            read.contains("(owner_id = '' OR owner_id = '00000000-0000-0000-0000-000000000007')"),
            "{read}"
        );
        assert!(sent.iter().all(|s| !s.trim_start().starts_with("INSERT")));
    }
}
