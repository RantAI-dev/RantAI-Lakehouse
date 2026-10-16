//! HTTP-level integration tests for `POST /api/ai/tool` (T0.4 of the
//! copilot-operations-handover plan) and its audit trail (T0.3's leftover
//! half): the SAME `gate::decide` path the chat loop runs, exercised
//! through the real router with a real Postgres-backed `audit_event`
//! table, so these prove "one audit row per call, with the right
//! outcome" end to end rather than only at the unit level (see
//! `crates/lakehouse-api/src/routes/ai/gate.rs` and `.../audit.rs` for
//! those).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use lakehouse_store::audit::{AuditFilter, list};
use serde_json::{Value, json};
use tower::ServiceExt;

use common::{TestApp, session_cookie_for_seeded_user, spin_up};

async fn post_tool(
    app: &axum::Router,
    cookie: Option<&str>,
    body: Value,
) -> axum::http::Response<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/api/ai/tool")
        .header("content-type", "application/json");
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    app.clone()
        .oneshot(
            builder
                .body(Body::from(
                    serde_json::to_vec(&body).expect("serialize body"),
                ))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright")
}

async fn json_body(resp: axum::http::Response<Body>) -> Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read response body");
    serde_json::from_slice(&bytes).expect("valid JSON body")
}

/// Ask mode never executes a non-`Read` tool, even through
/// `POST /api/ai/tool` — the same D3 invariant the chat loop enforces, now
/// proven at the HTTP layer too. Exactly one `audit_event` row is written,
/// `outcome = "refused"`.
#[tokio::test]
async fn ask_mode_refuses_write_low_tool_and_audits_it() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let resp = post_tool(
        &router,
        Some(&cookie),
        json!({ "tool": "create_board", "args": { "name": "Wisatawan" }, "mode": "ask" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["outcome"], json!("refused"));
    assert_eq!(body["result"]["refused"], json!(true));
    assert!(
        body["result"].get("reason").is_none(),
        "ask-mode refusal carries no permission reason"
    );

    let events = list(&pool, AuditFilter::default())
        .await
        .expect("list audit events");
    let ours: Vec<_> = events
        .iter()
        .filter(|e| e.action == "create_board")
        .collect();
    assert_eq!(ours.len(), 1, "exactly one audit row for this call");
    assert_eq!(ours[0].outcome, "refused");
    assert_eq!(ours[0].principal_kind.as_deref(), Some("copilot"));
}

/// A principal lacking the tool's permission is refused via
/// `POST /api/ai/tool`, with the same `reason: "permission"` shape the
/// chat loop's gate produces, and exactly one audit row.
#[tokio::test]
async fn missing_permission_refuses_and_audits_with_permission_reason() {
    let TestApp { router, pool } = spin_up().await;
    let user_id = common::create_zero_permission_principal(&pool).await;
    let cookie = common::session_cookie_for_user(&pool, user_id).await;

    let resp = post_tool(
        &router,
        Some(&cookie),
        json!({ "tool": "create_chart", "args": { "title": "T", "kind": "bar", "confirmed": true }, "mode": "build" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["outcome"], json!("refused"));
    assert_eq!(body["result"]["reason"], json!("permission"));
    assert_eq!(body["result"]["required"], json!("dashboard:write"));

    let events = list(&pool, AuditFilter::default())
        .await
        .expect("list audit events");
    let ours: Vec<_> = events
        .iter()
        .filter(|e| e.action == "create_chart")
        .collect();
    assert_eq!(ours.len(), 1);
    assert_eq!(ours[0].outcome, "refused");
}

/// SEC-9-AC2 through the real router: a principal without `query:read` is
/// refused by both SQL tools, the permission is named, and the audited
/// outcome is `refused`, not `failed`. The test app's `ClickHouse` upstream is
/// deliberately dead, so a call that had reached an engine would have been
/// audited `failed`; `refused` is the proof that nothing was sent.
#[tokio::test]
async fn the_sql_tools_are_refused_without_query_read_and_never_reach_an_engine() {
    let TestApp { router, pool } = spin_up().await;
    let user_id = common::create_zero_permission_principal(&pool).await;
    let cookie = common::session_cookie_for_user(&pool, user_id).await;

    for (tool, args) in [
        ("run_sql", json!({ "sql": "SELECT 1" })),
        ("run_saved_query", json!({ "id": "q-planted" })),
    ] {
        let resp = post_tool(
            &router,
            Some(&cookie),
            json!({ "tool": tool, "args": args, "mode": "build" }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK, "{tool}");
        let body = json_body(resp).await;
        assert_eq!(body["outcome"], json!("refused"), "{tool}");
        assert_eq!(body["result"]["reason"], json!("permission"), "{tool}");
        assert_eq!(body["result"]["required"], json!("query:read"), "{tool}");
    }
}

/// A `WriteLow` tool called in build mode WITHOUT `confirmed: true`
/// returns `needs_confirmation` and executes nothing — proven here by the
/// audited outcome, since the real `create_board` execution would hit the
/// (deliberately dead) `ClickHouse` upstream and come back `failed`
/// instead, which this test asserts it never reaches.
#[tokio::test]
async fn write_low_tool_without_confirmed_needs_confirmation_and_never_executes() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let resp = post_tool(
        &router,
        Some(&cookie),
        json!({ "tool": "create_board", "args": { "name": "Wisatawan" }, "mode": "build" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["outcome"], json!("needs_confirmation"));
    assert_eq!(body["result"]["needs_confirmation"], json!(true));
    assert_eq!(body["result"]["tool"], json!("create_board"));
    assert!(
        body["result"]["summary"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "must carry a non-empty human-readable summary"
    );

    let events = list(&pool, AuditFilter::default())
        .await
        .expect("list audit events");
    let ours: Vec<_> = events
        .iter()
        .filter(|e| e.action == "create_board")
        .collect();
    assert_eq!(ours.len(), 1);
    assert_eq!(ours[0].outcome, "needs_confirmation");
}

/// The SAME call, resent with `confirmed: true`, passes the gate — it
/// reaches `run_tool` (proven by the outcome no longer being `refused` or
/// `needs_confirmation`; it becomes `failed` here only because the test
/// harness points `ClickHouse` at a dead upstream, not because the gate
/// blocked it) — and is still audited exactly once, distinctly from the
/// unconfirmed attempt above.
#[tokio::test]
async fn write_low_tool_confirmed_passes_the_gate_and_is_audited_as_executed_or_failed() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let resp = post_tool(
        &router,
        Some(&cookie),
        json!({ "tool": "create_board", "args": { "name": "Wisatawan", "confirmed": true }, "mode": "build" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_ne!(body["outcome"], json!("refused"));
    assert_ne!(body["outcome"], json!("needs_confirmation"));

    let events = list(&pool, AuditFilter::default())
        .await
        .expect("list audit events");
    let ours: Vec<_> = events
        .iter()
        .filter(|e| e.action == "create_board")
        .collect();
    assert_eq!(
        ours.len(),
        1,
        "exactly one audit row for the confirmed call"
    );
    assert_ne!(ours[0].outcome, "refused");
    assert_ne!(ours[0].outcome, "needs_confirmation");
}

/// `WriteHigh` (`delete_chart`) is NEVER executed by `POST /api/ai/tool`,
/// even with `confirmed: true` (that flag only means something for
/// `WriteLow`) — instead it creates a pending `agent_run` +
/// `approval_item` pair (T0.5) and is audited `needs_approval`, linked by
/// `run_id`/`approval_id`.
#[tokio::test]
async fn write_high_tool_creates_a_pending_approval_and_never_executes() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let resp = post_tool(
        &router,
        Some(&cookie),
        json!({ "tool": "delete_chart", "args": { "id": "c-1", "confirmed": true }, "mode": "build" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["outcome"], json!("needs_approval"));
    assert_eq!(body["result"]["needs_approval"], json!(true));
    assert_eq!(body["result"]["tool"], json!("delete_chart"));
    let approval_id = body["result"]["approval_id"]
        .as_str()
        .expect("approval_id present")
        .to_owned();
    let run_id = body["result"]["run_id"]
        .as_str()
        .expect("run_id present")
        .to_owned();

    // Exactly one approval + one run, and NOTHING executed (the chart
    // still doesn't exist, but more importantly there is no way to tell
    // from this response alone — the run's own status proves it).
    let approvals = lakehouse_store::agents::list_approvals(&pool, None)
        .await
        .expect("list approvals");
    assert_eq!(approvals.len(), 1);
    assert_eq!(approvals[0].id, approval_id);
    assert_eq!(approvals[0].status, "pending");
    assert_eq!(approvals[0].action, "delete_chart");

    let run = lakehouse_store::agents::get_run(&pool, &run_id)
        .await
        .expect("get run")
        .expect("run exists");
    assert_eq!(run.status, "waiting_approval");
    assert_eq!(run.employee_id, "emp-copilot");

    let events = list(&pool, AuditFilter::default())
        .await
        .expect("list audit events");
    let ours: Vec<_> = events
        .iter()
        .filter(|e| e.action == "delete_chart")
        .collect();
    assert_eq!(ours.len(), 1);
    assert_eq!(ours[0].outcome, "needs_approval");
    assert_eq!(ours[0].run_id.as_deref(), Some(run_id.as_str()));
    assert_eq!(ours[0].approval_id.as_deref(), Some(approval_id.as_str()));
}

/// Plan invariant 5: a secret-shaped argument is never stored in
/// `audit_event.args`, even though it is happily accepted (and refused for
/// an unrelated reason) at the HTTP layer.
#[tokio::test]
async fn secret_shaped_args_are_redacted_before_they_reach_the_audit_row() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let resp = post_tool(
        &router,
        Some(&cookie),
        json!({
            "tool": "create_board",
            "args": { "name": "Wisatawan", "apiKey": "sk-super-secret" },
            "mode": "build",
        }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let events = list(&pool, AuditFilter::default())
        .await
        .expect("list audit events");
    let ours = events
        .iter()
        .find(|e| e.action == "create_board")
        .expect("an audit row for this call");
    assert_eq!(ours.args["apiKey"], json!("[redacted]"));
    assert_eq!(ours.args["name"], json!("Wisatawan"));
}

/// An unregistered tool name is a plain 400, not a 500 or a silent pass —
/// and it is never audited (there is no `ToolSpec` to attribute the
/// action to).
#[tokio::test]
async fn unknown_tool_name_is_a_400_and_is_not_audited() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let resp = post_tool(
        &router,
        Some(&cookie),
        json!({ "tool": "not_a_real_tool", "args": {}, "mode": "build" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let events = list(&pool, AuditFilter::default())
        .await
        .expect("list audit events");
    assert!(events.iter().all(|e| e.action != "not_a_real_tool"));
}

/// `POST /api/ai/tool` is `Policy::RequiresAuth` — an unauthenticated
/// request is refused before the handler (and the gate) ever runs.
#[tokio::test]
async fn unauthenticated_request_is_refused_by_the_policy_layer() {
    let TestApp { router, .. } = spin_up().await;
    let resp = post_tool(
        &router,
        None,
        json!({ "tool": "create_board", "args": {}, "mode": "build" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ── AI-1 / AI-2 / AI-20 (plan 20): dashboard-board tools ────────────────
//
// Every test in this section is an HTTP-level exercise of `POST
// /api/ai/tool` for one of the six new tools. The harness points
// `ClickHouse` at a dead upstream (port 1 — a TCP RST, no DNS, no
// wait: see `tests/common/mod.rs`), so a tool whose success path runs
// a route that touches ClickHouse (`rename_board`, `delete_board`,
// `arrange_board`, `move_chart`, `save_board_filters`) lands at
// `outcome: "failed"` with the upstream error in the result. What each
// test here CAN still prove without a live ClickHouse:
//
// - the gate's decision: refusal, `needs_confirmation`, or `failed`;
// - the route's own CPU-side refusals (the `board_edit_allowed`
//   "invalid dashboard" 400 on the built-in Main board, the tool's
//   own "id/board/name/filters required" pre-checks);
// - the audit row's `action` and `outcome`;
// - the `Read` view-filter payload, which is built locally and never
//   reaches ClickHouse.
//
// Plan 20's end-to-end state-change assertions are NOT in this file:
// they need real `bi_board` / `bi_chart` rows in ClickHouse, which
// the harness cannot produce, and are recorded as gaps here so a
// reader is not left guessing:
//
//   * `move_chart` (plan 132): "the chart's board changes, its tile
//     leaves the old board's layout, its other fields are unchanged"
//     — needs a real `bi_chart` row (to read+re-write the chart) and a
//     real `bi_board` row (to remove the tile from the old board's
//     layout). The tool's first store call is `list_stored_charts`,
//     which the dead upstream turns into a 502-class failure before
//     any "chart not found" CPU check is reachable.
//   * `arrange_board` (plan 130-131): "the listed tile moves, the
//     others keep their boxes" and "a chart from another board is
//     refused" — both need a real `bi_board` row (to read the current
//     layout, merge the new tiles, and save the merged layout) and a
//     real `bi_chart` row (to check the off-board chart id is on the
//     named board). The CPU-only bounds / off-grid /
//     `w`-or-`h`-below-1 checks are covered by `parse_arrange_tiles`'s
//     unit tests in `tools/boards.rs`.
//   * `save_board_filters` (plan 133): of its four lines, three are covered
//     here — "21 filters are refused" and "an invalid op is refused" both run
//     before any `ClickHouse` call (see
//     `save_board_filters_refuses_more_than_twenty_filters` and
//     `save_board_filters_refuses_an_invalid_op`), and "Main refuses" runs
//     inside `board_edit_allowed` (see
//     `save_board_filters_on_main_fails_with_the_console_message`). Only
//     "valid filters are saved" is a gap: it needs a real `bi_board` row
//     whose `filters_json` the call would write through
//     `store::update_board_filters`.
//   * `set_board_view_filters` / `AI-2-AC2` (plan 134): "the board's
//     saved filters are unchanged after the call" needs to read
//     `bi_board.filters_json` before and after to establish a
//     baseline. The tool itself is local: `pub(super) fn
//     set_board_view_filters(args) -> Value` (no async, no `&state`,
//     no ClickHouse ref) and only its returned payload is observable
//     through the HTTP layer.
//
// No `#[ignore]` placeholders are left for this work, because each
// gap is a real fixture (a `bi_board` row, a `bi_chart` row, a
// layout) this harness cannot produce, not a skipped assertion.

/// `rename_board` (WriteLow): without `confirmed: true` the gate returns
/// `needs_confirmation` and nothing reaches the route.
#[tokio::test]
async fn rename_board_without_confirmed_is_a_draft_and_nothing_changed() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let resp = post_tool(
        &router,
        Some(&cookie),
        json!({
            "tool": "rename_board",
            "args": { "id": "b-rename", "name": "Renamed" },
            "mode": "build"
        }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["outcome"], json!("needs_confirmation"));
    assert_eq!(body["result"]["needs_confirmation"], json!(true));
    assert!(
        body["result"]["summary"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "needs_confirmation must carry a non-empty summary"
    );
    assert!(
        body["result"]["summary"]
            .as_str()
            .is_some_and(|s| s.contains("b-rename") && s.contains("Renamed")),
        "summary must name the board and the new name, got: {}",
        body["result"]["summary"]
    );
}

/// `rename_board` with `confirmed: true` passes the gate and reaches the
/// route; the test harness's `ClickHouse` is a dead upstream, so the
/// run ends `failed` — but the gate's decision is what the test pins.
#[tokio::test]
async fn rename_board_confirmed_passes_the_gate_and_is_audited_as_failed() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let resp = post_tool(
        &router,
        Some(&cookie),
        json!({
            "tool": "rename_board",
            "args": { "id": "b-rename", "name": "Renamed", "confirmed": true },
            "mode": "build"
        }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_ne!(body["outcome"], json!("refused"));
    assert_ne!(body["outcome"], json!("needs_confirmation"));
    let events = list(&pool, AuditFilter::default())
        .await
        .expect("list audit events");
    let ours: Vec<_> = events
        .iter()
        .filter(|e| e.action == "rename_board")
        .collect();
    assert_eq!(ours.len(), 1, "exactly one audit row");
    assert_ne!(ours[0].outcome, "refused");
    assert_ne!(ours[0].outcome, "needs_confirmation");
    // AI-1: the resource is the board.
    assert_eq!(ours[0].resource_kind.as_deref(), Some("board"));
    assert_eq!(ours[0].resource_id.as_deref(), Some("b-rename"));
}

/// `rename_board` on the built-in Main board is refused by the route's
/// own `board_edit_allowed`. The test harness's `ClickHouse` is dead, so
/// the "invalid dashboard" message comes back from the route (no row
/// was ever written there for this board id, but the route's check runs
/// before any clickhouse call, so the 400 surfaces cleanly).
#[tokio::test]
async fn rename_board_on_default_fails_with_the_console_message() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let resp = post_tool(
        &router,
        Some(&cookie),
        json!({
            "tool": "rename_board",
            "args": { "id": "default", "name": "Anything", "confirmed": true },
            "mode": "build"
        }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_ne!(body["outcome"], json!("needs_confirmation"));
    assert_ne!(body["outcome"], json!("refused"));
    assert_eq!(
        body["result"]["error"],
        json!("invalid dashboard"),
        "route's own message must surface through the tool"
    );
}

/// `arrange_board` (WriteLow): the unconfirmed call needs confirmation.
#[tokio::test]
async fn arrange_board_without_confirmed_needs_confirmation() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let resp = post_tool(
        &router,
        Some(&cookie),
        json!({
            "tool": "arrange_board",
            "args": {
                "board": "b-arrange",
                "tiles": { "c-1": { "x": 0, "y": 0, "w": 6, "h": 3 } }
            },
            "mode": "build"
        }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["outcome"], json!("needs_confirmation"));
    assert!(
        body["result"]["summary"]
            .as_str()
            .is_some_and(|s| s.contains("1 tile") && s.contains("b-arrange")),
        "summary must name the board and the count, got: {}",
        body["result"]["summary"]
    );
}

/// `save_board_filters` (WriteLow): the unconfirmed call needs
/// confirmation, and the summary names the board and the filter count.
#[tokio::test]
async fn save_board_filters_without_confirmed_needs_confirmation() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let filters = json!([
        { "column": "kabupaten", "values": ["A", "B"] },
        { "column": "tahun", "values": ["2026"] }
    ]);
    let resp = post_tool(
        &router,
        Some(&cookie),
        json!({
            "tool": "save_board_filters",
            "args": { "board": "b-filters", "filters": filters },
            "mode": "build"
        }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["outcome"], json!("needs_confirmation"));
    assert!(
        body["result"]["summary"]
            .as_str()
            .is_some_and(|s| s.contains("2 filter") && s.contains("b-filters")),
        "summary must name the board and the count, got: {}",
        body["result"]["summary"]
    );
}

/// `save_board_filters` on the built-in Main board is refused by the
/// route's own `board_edit_allowed`. The test harness's `ClickHouse` is
/// dead, but the route's check runs before any `ClickHouse` call (same
/// shape as `rename_board_on_default_fails_with_the_console_message`),
/// so the 400 surfaces cleanly. The "valid filters are saved" half of
/// the plan's Save-filters line is not in this file: it would write to
/// `bi_board.filters_json` and the harness cannot observe that — see
/// this section's header.
#[tokio::test]
async fn save_board_filters_on_main_fails_with_the_console_message() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let resp = post_tool(
        &router,
        Some(&cookie),
        json!({
            "tool": "save_board_filters",
            "args": {
                "board": "default",
                "filters": [{ "column": "kabupaten", "values": ["A"] }],
                "confirmed": true
            },
            "mode": "build"
        }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_ne!(body["outcome"], json!("needs_confirmation"));
    assert_ne!(body["outcome"], json!("refused"));
    assert_eq!(
        body["result"]["error"],
        json!("invalid dashboard"),
        "route's own message must surface through the tool"
    );
}

/// `save_board_filters` refuses more than the 20 filters the route allows
/// (`routes/dashboard.rs:942`, `validate_filters`). The refusal runs before
/// the route's first `ClickHouse` call, so the dead upstream never gets
/// asked — this is the plan's "21 filters are refused" line, asserted
/// through the tool rather than against the library.
#[tokio::test]
async fn save_board_filters_refuses_more_than_twenty_filters() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let too_many: Vec<_> = (0..21)
        .map(|i| json!({ "column": format!("c{i}"), "values": ["A"] }))
        .collect();
    let resp = post_tool(
        &router,
        Some(&cookie),
        json!({
            "tool": "save_board_filters",
            "args": { "board": "b-some", "filters": too_many, "confirmed": true },
            "mode": "build"
        }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_ne!(body["outcome"], json!("needs_confirmation"));
    assert_ne!(body["outcome"], json!("refused"));
    let err = body["result"]["error"].as_str().expect("error message");
    assert!(
        err.contains("at most 20"),
        "the route's own limit must surface through the tool, got: {err}"
    );
}

/// `save_board_filters` refuses an invalid filter op. The tool parses the
/// args into `Vec<FilterDef>` itself (`tools/boards.rs:419`), so this one
/// never reaches the route at all — it is the plan's "an invalid op is
/// refused" line for Save filters, which the view-filter tool's own invalid
/// op test does not cover.
#[tokio::test]
async fn save_board_filters_refuses_an_invalid_op() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let resp = post_tool(
        &router,
        Some(&cookie),
        json!({
            "tool": "save_board_filters",
            "args": {
                "board": "b-some",
                "filters": [{ "column": "kabupaten", "values": ["A"], "op": "fuzzy_match" }],
                "confirmed": true
            },
            "mode": "build"
        }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_ne!(body["outcome"], json!("needs_confirmation"));
    assert_ne!(body["outcome"], json!("refused"));
    let err = body["result"]["error"].as_str().expect("error message");
    // The parse error is always prefixed `filters: `, so that proves
    // nothing; naming the op the model actually sent is the part that matters.
    assert!(
        err.contains("fuzzy_match"),
        "the refusal must name the unknown op, got: {err}"
    );
}

/// `set_board_view_filters` (Read): passes the gate in any mode; the
/// tool is local, so the result is the validated view-filter payload
/// the page will apply — `applyViewFilters: true`, the board id, and
/// the cleaned filter list. What the test proves is the returned
/// shape; the "nothing was saved" half of that claim is a property of
/// the tool's implementation, not something observable through this
/// harness (the saved `bi_board.filters_json` row is in `ClickHouse`,
/// and the harness's `ClickHouse` is a dead upstream — see this
/// section's header).
#[tokio::test]
async fn set_board_view_filters_returns_the_view_payload() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let filters = json!([
        { "column": "kabupaten", "values": ["A"] }
    ]);
    let resp = post_tool(
        &router,
        Some(&cookie),
        json!({
            "tool": "set_board_view_filters",
            "args": { "board": "b-view", "filters": filters },
            "mode": "ask"
        }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_ne!(body["outcome"], json!("refused"));
    assert_ne!(body["outcome"], json!("needs_confirmation"));
    let result = &body["result"];
    assert_eq!(result["applyViewFilters"], json!(true));
    assert_eq!(result["board"], json!("b-view"));
    assert_eq!(result["filters"][0]["column"], json!("kabupaten"));
}

/// `set_board_view_filters` is available to a Dashboard Viewer (only
/// `dashboard:read`); the five writing tools are refused with the
/// permission message.
#[tokio::test]
async fn set_board_view_filters_works_with_dashboard_read_only() {
    let TestApp { router, pool } = spin_up().await;
    let user_id = common::create_principal_with_permissions(&pool, "dashboard:read").await;
    let cookie = common::session_cookie_for_user(&pool, user_id).await;

    // The view filter call passes the gate; the route is not called, so
    // there is no ClickHouse dependency. Outcome is `executed` (no
    // `error` key in the result), the body has the page payload.
    let resp = post_tool(
        &router,
        Some(&cookie),
        json!({
            "tool": "set_board_view_filters",
            "args": { "board": "b-view", "filters": [] },
            "mode": "build"
        }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_ne!(body["outcome"], json!("refused"));
    assert_eq!(body["result"]["applyViewFilters"], json!(true));

    // Every writing tool (and the high-risk `delete_board`) is refused
    // with `reason: "permission"` and the required permission named.
    for (tool, required) in [
        ("rename_board", "dashboard:write"),
        ("delete_board", "dashboard:write"),
        ("arrange_board", "dashboard:write"),
        ("move_chart", "dashboard:write"),
        ("save_board_filters", "dashboard:write"),
    ] {
        let resp = post_tool(
            &router,
            Some(&cookie),
            json!({
                "tool": tool,
                "args": { "id": "x", "name": "y" },
                "mode": "build"
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK, "tool {tool}");
        let body = json_body(resp).await;
        assert_eq!(body["outcome"], json!("refused"), "tool {tool}");
        assert_eq!(body["result"]["reason"], json!("permission"), "tool {tool}");
        assert_eq!(body["result"]["required"], json!(required), "tool {tool}");
    }
}

/// A principal with no permissions at all is refused on every new tool,
/// including the `Read` view filter (which needs `dashboard:read`).
#[tokio::test]
async fn new_tools_are_refused_for_a_zero_permission_principal() {
    let TestApp { router, pool } = spin_up().await;
    let user_id = common::create_zero_permission_principal(&pool).await;
    let cookie = common::session_cookie_for_user(&pool, user_id).await;

    for tool in [
        "rename_board",
        "delete_board",
        "arrange_board",
        "move_chart",
        "save_board_filters",
        "set_board_view_filters",
    ] {
        let resp = post_tool(
            &router,
            Some(&cookie),
            json!({ "tool": tool, "args": {}, "mode": "build" }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK, "tool {tool}");
        let body = json_body(resp).await;
        assert_eq!(body["outcome"], json!("refused"), "tool {tool}");
        assert_eq!(body["result"]["reason"], json!("permission"), "tool {tool}");
    }
}

/// Ask mode refuses the five writing tools and allows
/// `set_board_view_filters` (the only one whose `Risk::Read` keeps it
/// out of the Ask-mode gate).
#[tokio::test]
async fn ask_mode_refuses_the_five_writers_and_allows_set_board_view_filters() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    for tool in [
        "rename_board",
        "delete_board",
        "arrange_board",
        "move_chart",
        "save_board_filters",
    ] {
        let resp = post_tool(
            &router,
            Some(&cookie),
            json!({
                "tool": tool,
                "args": { "id": "x", "name": "y" },
                "mode": "ask"
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK, "tool {tool}");
        let body = json_body(resp).await;
        assert_eq!(body["outcome"], json!("refused"), "tool {tool}");
        assert!(
            body["result"]["error"]
                .as_str()
                .is_some_and(|s| s.contains("Ask mode")),
            "tool {tool}: ask-mode refusal must mention Ask mode, got {}",
            body["result"]["error"]
        );
    }

    // The view filter call still passes in Ask mode.
    let resp = post_tool(
        &router,
        Some(&cookie),
        json!({
            "tool": "set_board_view_filters",
            "args": { "board": "b-view", "filters": [] },
            "mode": "ask"
        }),
    )
    .await;
    let body = json_body(resp).await;
    assert_ne!(body["outcome"], json!("refused"));
}
