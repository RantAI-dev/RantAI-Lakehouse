//! The four signed-embed tools through `POST /api/ai/tool` and the approvals
//! route (`BI-26` slice 2 T5, D4 to D9, with `AI-11-AC3`): the requester's
//! manage check runs before an approval exists, an approved `enable_embed` adds
//! sites to the ones already listed and never removes one, an approver without
//! manage is recorded and nothing runs, and no result ever carries a token,
//! the embedding secret or a `sampleToken`.
//!
//! Postgres holds the rules and the approvals (a per-test database); a
//! `wiremock` stand-in plays `ClickHouse`, keeping the board rows and the
//! append-only `bi_board_embed_event` rows the tools write, so a later read
//! sees what a write stored.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{
    TestApp, create_assistant_principal, create_principal_with_permissions,
    session_cookie_for_user, spin_up_with_env,
};
use lakehouse_embed::{EmbedClaims, EmbedResource, sign_embed, unix_now};
use lakehouse_store::dashboard_access::{NewRule, replace_item};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request as MockRequest, Respond, ResponseTemplate};

/// The test's embedding secret. Not a real secret, and the same one
/// `tests/embed_tokens.rs` uses.
const SECRET: &str = "test-embed-secret-not-real";

/// The sentence `routes::embed::EMBEDDING_NOT_CONFIGURED` carries.
const NOT_CONFIGURED: &str = "embedding is not configured";

/// The routes' sentence for a locked or editable filter that does not pick
/// values (`ONLY_VALUE_FILTERS`).
const ONLY_VALUE_FILTERS: &str = "Only a filter that picks values can be locked or editable in an embed. Leave the others hidden.";

/// The routes' sentence for a mode on a column the dashboard has no filter for.
const NOT_A_FILTER: &str = "A mode can only be set for a filter the dashboard has.";

/// The dashboards the stand-in holds: `(id, embed_enabled)`.
const BOARDS: [(&str, &str); 2] = [("b1", "0"), ("b2", "1")];

/// The saved filters of `b1`: one that picks values, and a range.
const FILTERS: &str = r#"[{"column":"region","op":"in","values":["Bali"]},{"column":"year","op":"between","min":"2020","max":"2024"}]"#;

/// A recording `ClickHouse` stand-in that keeps the board rows and the embed
/// events, so a write the tools make is visible to a later read.
#[derive(Clone)]
struct Store {
    boards: Arc<Mutex<Vec<Value>>>,
    events: Arc<Mutex<Vec<Value>>>,
}

impl Store {
    fn new(events: Vec<Value>) -> Self {
        let boards = BOARDS
            .iter()
            .map(|(id, enabled)| {
                json!({
                    "id": id, "name": format!("Board {id}"), "description": "",
                    "created_by": "test", "layout_json": "", "filters_json": FILTERS,
                    "public_token": format!("p_{id}"), "embed_enabled": enabled,
                    "folder_id": "", "owner_id": "", "created_at": "2026-10-01 00:00:00",
                })
            })
            .collect();
        Self {
            boards: Arc::new(Mutex::new(boards)),
            events: Arc::new(Mutex::new(events)),
        }
    }

    /// The origin event every test starts from: one site was already allowed.
    fn with_earlier_site() -> Self {
        Self::new(vec![json!({
            "board_id": "b1", "kind": "origins", "revoked_before": "0",
            "jti": "", "exp": "0",
            "origins_json": r#"["https://earlier.example"]"#,
        })])
    }

    fn set_enabled(&self, id: &str, enabled: bool) {
        for board in self.boards.lock().unwrap().iter_mut() {
            if board["id"] == id {
                board["embed_enabled"] = json!(if enabled { "1" } else { "0" });
            }
        }
    }

    /// Record the embed event an `INSERT INTO console.bi_board_embed_event`
    /// carries; the aggregate read answers from these rows.
    fn record_event(&self, sql: &str) {
        let Some(pos) = sql.find("VALUES (") else {
            return;
        };
        let values = split_top_level(&sql[pos + "VALUES (".len()..]);
        if values.len() < 3 {
            return;
        }
        let board_id = unquote(&values[0]);
        let kind = if sql.contains("'origins'") {
            "origins"
        } else if sql.contains("'revoke_all'") {
            "revoke_all"
        } else if sql.contains("'filter_modes'") {
            "filter_modes"
        } else {
            "withdraw"
        };
        let value = unquote(values.get(2).map_or("", String::as_str));
        let revoked_before = if kind == "revoke_all" {
            value.clone()
        } else {
            "0".to_owned()
        };
        let origins_json = if kind == "origins" || kind == "filter_modes" {
            value
        } else {
            "[]".to_owned()
        };
        self.events.lock().unwrap().push(json!({
            "board_id": board_id,
            "kind": kind,
            "revoked_before": revoked_before,
            "jti": "",
            "exp": "0",
            "origins_json": origins_json,
        }));
    }

    /// A board save carries `embed_enabled` as the eighth value
    /// (`store::upsert_board`'s full-row INSERT).
    fn record_board_insert(&self, sql: &str) {
        let Some(pos) = sql.find("VALUES (") else {
            return;
        };
        let values = split_top_level(&sql[pos + "VALUES (".len()..]);
        if values.len() < 8 {
            return;
        }
        let id = unquote(&values[0]);
        self.set_enabled(&id, values[7].trim() == "1");
    }
}

impl Respond for Store {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body).into_owned();
        let rows = |data: Vec<Value>| {
            let n = data.len();
            ResponseTemplate::new(200).set_body_json(json!({ "meta": [], "data": data, "rows": n }))
        };
        if sql.starts_with("INSERT INTO console.bi_board (") {
            self.record_board_insert(&sql);
            return ResponseTemplate::new(200).set_body_string("Ok.\n");
        }
        if sql.starts_with("INSERT INTO console.bi_board_embed_event") {
            self.record_event(&sql);
            return ResponseTemplate::new(200).set_body_string("Ok.\n");
        }
        if sql.contains("FROM console.bi_board_embed_event") {
            let found: Vec<Value> = self
                .events
                .lock()
                .unwrap()
                .iter()
                .filter(|e| {
                    sql.contains(&format!(
                        "board_id='{}'",
                        unquote(e["board_id"].as_str().unwrap_or_default())
                    ))
                })
                .cloned()
                .collect();
            return rows(found);
        }
        if sql.contains("SELECT id, folder_id, owner_id FROM console.bi_board") {
            return rows(
                self.boards
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|b| json!({ "id": b["id"], "folder_id": b["folder_id"], "owner_id": b["owner_id"] }))
                    .collect(),
            );
        }
        if sql.contains("FROM console.bi_board") {
            let wanted = sql.find("id='").map(|at| {
                let rest = &sql[at + "id='".len()..];
                rest[..rest.find('\'').unwrap_or(rest.len())].to_owned()
            });
            return rows(
                self.boards
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|b| wanted.as_ref().is_none_or(|w| b["id"] == w.as_str()))
                    .cloned()
                    .collect(),
            );
        }
        // Folders, charts and every other read: nothing.
        rows(Vec::new())
    }
}

/// Split a SQL value list at top-level commas, keeping quoted commas together.
fn split_top_level(text: &str) -> Vec<String> {
    let text = text.trim_end().trim_end_matches(')');
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            current.push(c);
            if c == '\\' {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            } else if c == '\'' {
                quoted = false;
            }
        } else if c == '\'' {
            quoted = true;
            current.push(c);
        } else if c == ',' {
            parts.push(current.trim().to_owned());
            current.clear();
        } else {
            current.push(c);
        }
    }
    if !current.trim().is_empty() {
        parts.push(current.trim().to_owned());
    }
    parts
}

fn unquote(value: &str) -> String {
    value
        .trim()
        .trim_matches('\'')
        .replace("\\\"", "\"")
        .replace("''", "'")
}

async fn app(secret: Option<&str>, store: Store) -> (TestApp, MockServer) {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(store)
        .mount(&ch)
        .await;
    let mut env: HashMap<String, String> = HashMap::from([("CH_URL".to_owned(), ch.uri())]);
    if let Some(secret) = secret {
        env.insert("EMBED_SECRET".to_owned(), secret.to_owned());
    }
    env.insert(
        "CATALOG_TENANT_ID".to_owned(),
        common::CATALOG_TENANT.to_owned(),
    );
    (spin_up_with_env(&env).await, ch)
}

fn manage(user: Uuid) -> NewRule {
    NewRule {
        user_id: Some(user),
        role_id: None,
        level: "manage".to_owned(),
    }
}

/// A manager through a rule alone: `assistant:use` (every call here is
/// `POST /api/ai/tool`) and no dashboard permission; `manage` on `b1` and
/// `b2` through their own rules.
async fn manager(app: &TestApp) -> (Uuid, String) {
    let user = create_assistant_principal(&app.pool, "").await;
    for board in ["b1", "b2"] {
        replace_item(&app.pool, "board", board, &[manage(user)], false, None)
            .await
            .unwrap();
    }
    let cookie = session_cookie_for_user(&app.pool, user).await;
    (user, cookie)
}

async fn call(app: &TestApp, cookie: &str, tool: &str, args: Value, mode: &str) -> Value {
    let resp = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ai/tool")
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(
                    json!({ "tool": tool, "args": args, "mode": mode }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

async fn decide(app: &TestApp, cookie: &str, id: &str) -> (StatusCode, Value) {
    let resp = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agents/approvals/{id}/decide"))
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(json!({ "decision": "approved" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn approvals_for(app: &TestApp, action: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM approval_item WHERE action = $1")
        .bind(action)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

async fn approval_reason(app: &TestApp, id: &str) -> String {
    sqlx::query_scalar("SELECT coalesce(reason, '') FROM approval_item WHERE id = $1")
        .bind(id)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

/// An approver: `agent:approve`, and `manage` on the named dashboards through
/// their own rules. `also` keeps the requester's rules, because
/// `replace_item` replaces an item's whole rule list.
async fn approver(app: &TestApp, boards: &[&str], also: &[Uuid]) -> String {
    let user = create_principal_with_permissions(&app.pool, "agent:approve").await;
    for board in boards {
        let mut rules: Vec<NewRule> = also.iter().map(|u| manage(*u)).collect();
        rules.push(manage(user));
        replace_item(&app.pool, "board", board, &rules, false, None)
            .await
            .unwrap();
    }
    session_cookie_for_user(&app.pool, user).await
}

/// A JWT's shape: three base64url segments of at least eight characters each.
fn has_jwt_shape(text: &str) -> bool {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')))
        .any(|part| {
            let segments: Vec<&str> = part.split('.').collect();
            segments.len() == 3 && segments.iter().all(|s| s.len() >= 8)
        })
}

fn assert_no_secret(result: &Value) {
    let text = result.to_string();
    assert!(!has_jwt_shape(&text), "a JWT-shaped string: {text}");
    assert!(!text.contains(SECRET), "the embedding secret: {text}");
    assert!(!text.contains("sampleToken"), "a sample token: {text}");
}

// ── The requester's own checks (D4, D5) ────────────────────────────────

#[tokio::test]
async fn a_requester_without_manage_is_refused_and_no_approval_is_written() {
    let (app, _ch) = app(Some(SECRET), Store::with_earlier_site()).await;
    let viewer = create_assistant_principal(&app.pool, "dashboard:read").await;
    let cookie = session_cookie_for_user(&app.pool, viewer).await;
    for (tool, args) in [
        (
            "enable_embed",
            json!({ "board": "b1", "sites": ["https://example.com"] }),
        ),
        (
            "set_embed_filter_modes",
            json!({ "board": "b1", "modes": { "region": "locked" } }),
        ),
    ] {
        let body = call(&app, &cookie, tool, args, "build").await;
        let error = body["result"]["error"].as_str().unwrap_or_default();
        assert!(error.contains("manage on this dashboard"), "{body}");
        assert_eq!(body["outcome"], "failed", "{body}");
        assert_eq!(approvals_for(&app, tool).await, 0, "{body}");
    }
}

#[tokio::test]
async fn without_embed_secret_all_four_are_refused_with_the_routes_sentence() {
    let (app, _ch) = app(None, Store::with_earlier_site()).await;
    let (_user, cookie) = manager(&app).await;
    for (tool, args) in [
        (
            "enable_embed",
            json!({ "board": "b1", "sites": ["https://example.com"] }),
        ),
        (
            "set_embed_filter_modes",
            json!({ "board": "b1", "modes": { "region": "locked" } }),
        ),
        (
            "withdraw_embed_tokens",
            json!({ "board": "b1", "confirmed": true }),
        ),
        ("get_embed_snippet", json!({ "board": "b1" })),
    ] {
        let body = call(&app, &cookie, tool, args, "build").await;
        assert_eq!(body["result"]["error"], NOT_CONFIGURED, "{body}");
    }
    assert_eq!(approvals_for(&app, "enable_embed").await, 0);
    assert_eq!(approvals_for(&app, "set_embed_filter_modes").await, 0);
}

#[tokio::test]
async fn an_invalid_site_and_a_twenty_first_site_are_refused_before_any_approval() {
    let (first, _ch) = app(Some(SECRET), Store::with_earlier_site()).await;
    let (_user, cookie) = manager(&first).await;

    let invalid = call(
        &first,
        &cookie,
        "enable_embed",
        json!({ "board": "b1", "sites": ["not a site"] }),
        "build",
    )
    .await;
    assert!(
        invalid["result"]["error"]
            .as_str()
            .unwrap_or_default()
            .contains("is not an allowed site"),
        "{invalid}"
    );

    // Twenty sites already listed, so one more is the twenty-first.
    let store = Store::new(vec![json!({
        "board_id": "b1", "kind": "origins", "revoked_before": "0",
        "jti": "", "exp": "0",
        "origins_json": serde_json::to_string(
            &(0..20).map(|i| format!("https://s{i}.example")).collect::<Vec<_>>(),
        ).unwrap(),
    })]);
    let (twenty, _ch) = app(Some(SECRET), store).await;
    let (_user, cookie) = manager(&twenty).await;
    let too_many = call(
        &twenty,
        &cookie,
        "enable_embed",
        json!({ "board": "b1", "sites": ["https://example.com"] }),
        "build",
    )
    .await;
    assert_eq!(
        too_many["result"]["error"],
        json!("at most 20 sites can be allowed per dashboard."),
        "{too_many}"
    );
    assert_eq!(approvals_for(&twenty, "enable_embed").await, 0);
}

#[tokio::test]
async fn set_embed_filter_modes_refuses_a_column_that_is_no_filter_and_locked_on_a_range() {
    let (app, _ch) = app(Some(SECRET), Store::with_earlier_site()).await;
    let (_user, cookie) = manager(&app).await;
    let unknown = call(
        &app,
        &cookie,
        "set_embed_filter_modes",
        json!({ "board": "b1", "modes": { "nope": "locked" } }),
        "build",
    )
    .await;
    assert_eq!(unknown["result"]["error"], json!(NOT_A_FILTER), "{unknown}");

    let range = call(
        &app,
        &cookie,
        "set_embed_filter_modes",
        json!({ "board": "b1", "modes": { "year": "locked" } }),
        "build",
    )
    .await;
    assert_eq!(
        range["result"]["error"],
        json!(ONLY_VALUE_FILTERS),
        "{range}"
    );
    assert_eq!(approvals_for(&app, "set_embed_filter_modes").await, 0);
}

#[tokio::test]
async fn ask_mode_refuses_the_three_writes() {
    let (app, _ch) = app(Some(SECRET), Store::with_earlier_site()).await;
    let (_user, cookie) = manager(&app).await;
    for (tool, args) in [
        (
            "enable_embed",
            json!({ "board": "b1", "sites": ["https://example.com"] }),
        ),
        (
            "set_embed_filter_modes",
            json!({ "board": "b1", "modes": { "region": "locked" } }),
        ),
        (
            "withdraw_embed_tokens",
            json!({ "board": "b1", "confirmed": true }),
        ),
    ] {
        let body = call(&app, &cookie, tool, args, "ask").await;
        assert!(body["result"]["error"].is_string(), "{body}");
        assert_eq!(approvals_for(&app, tool).await, 0, "{body}");
    }
}

// ── enable_embed (D6, AI-11-AC3) ───────────────────────────────────────

#[tokio::test]
async fn an_approved_enable_embed_turns_it_on_and_adds_the_site_to_the_earlier_ones() {
    let (app, _ch) = app(Some(SECRET), Store::with_earlier_site()).await;
    let (user, cookie) = manager(&app).await;
    let asked = call(
        &app,
        &cookie,
        "enable_embed",
        json!({ "board": "b1", "sites": ["https://example.com"] }),
        "build",
    )
    .await;
    let approval_id = asked["result"]["approval_id"]
        .as_str()
        .unwrap_or_else(|| panic!("no approval: {asked}"))
        .to_owned();
    assert_eq!(asked["result"]["needs_approval"], json!(true), "{asked}");
    let reason = approval_reason(&app, &approval_id).await;
    assert!(reason.contains("b1"), "{reason}");
    assert!(reason.contains("https://example.com"), "{reason}");
    assert_eq!(approvals_for(&app, "enable_embed").await, 1);

    let (status, decided) =
        decide(&app, &approver(&app, &["b1"], &[user]).await, &approval_id).await;
    assert_eq!(status, StatusCode::OK, "{decided}");
    assert_eq!(decided["executed"], json!(true), "{decided}");
    assert_eq!(decided["result"]["enabled"], json!(true), "{decided}");

    // D6: the sites the board had are still there, and the new one joined them.
    let snippet = call(
        &app,
        &cookie,
        "get_embed_snippet",
        json!({ "board": "b1" }),
        "build",
    )
    .await;
    assert_eq!(snippet["result"]["embedding"], json!(true), "{snippet}");
    let sites = snippet["result"]["sites"].as_array().unwrap();
    assert!(
        sites.contains(&json!("https://earlier.example")),
        "{snippet}"
    );
    assert!(sites.contains(&json!("https://example.com")), "{snippet}");
    assert_no_secret(&decided["result"]);
    assert_no_secret(&snippet["result"]);
}

#[tokio::test]
async fn an_approver_without_manage_is_recorded_and_the_enable_does_not_run() {
    let (app, _ch) = app(Some(SECRET), Store::with_earlier_site()).await;
    let (_user, cookie) = manager(&app).await;
    let asked = call(
        &app,
        &cookie,
        "enable_embed",
        json!({ "board": "b1", "sites": ["https://example.com"] }),
        "build",
    )
    .await;
    let approval_id = asked["result"]["approval_id"].as_str().unwrap();
    let run_id = asked["result"]["run_id"].as_str().unwrap();

    // `agent:approve` and `dashboard:read` but no rule on b1: it sees the
    // dashboard and does not manage it.
    let outsider =
        create_principal_with_permissions(&app.pool, "agent:approve,dashboard:read").await;
    let outsider_cookie = session_cookie_for_user(&app.pool, outsider).await;
    let (status, decided) = decide(&app, &outsider_cookie, approval_id).await;
    assert_eq!(status, StatusCode::OK, "{decided}");
    // The embed settings are managed as somebody with a dashboard level: an
    // approver who holds none is refused by the guard's own sentence.
    let error = decided["result"]["error"].as_str().unwrap_or_default();
    assert!(error.contains("manage on this dashboard"), "{decided}");
    let run = lakehouse_store::agents::get_run(&app.pool, run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(run.status, "failed", "the run is recorded, not run");
    // Nothing was allowed and nothing was switched on.
    let snippet = call(
        &app,
        &cookie,
        "get_embed_snippet",
        json!({ "board": "b1" }),
        "build",
    )
    .await;
    assert_eq!(
        snippet["result"]["sites"],
        json!(["https://earlier.example"]),
        "{snippet}"
    );
    assert_eq!(snippet["result"]["embedding"], json!(false), "{snippet}");
}

// ── set_embed_filter_modes ─────────────────────────────────────────────

#[tokio::test]
async fn set_embed_filter_modes_waits_for_approval_then_the_snippet_lists_the_locked_column() {
    let (app, _ch) = app(Some(SECRET), Store::with_earlier_site()).await;
    let (user, cookie) = manager(&app).await;
    let asked = call(
        &app,
        &cookie,
        "set_embed_filter_modes",
        json!({ "board": "b1", "modes": { "region": "locked" } }),
        "build",
    )
    .await;
    assert_eq!(asked["result"]["needs_approval"], json!(true), "{asked}");
    let reason = approval_reason(&app, asked["result"]["approval_id"].as_str().unwrap()).await;
    assert!(reason.contains("b1"), "{reason}");
    assert!(reason.contains("region locked"), "{reason}");

    let (status, decided) = decide(
        &app,
        &approver(&app, &["b1"], &[user]).await,
        asked["result"]["approval_id"].as_str().unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{decided}");
    assert_eq!(decided["executed"], json!(true), "{decided}");
    assert_no_secret(&decided["result"]);
    let snippet = call(
        &app,
        &cookie,
        "get_embed_snippet",
        json!({ "board": "b1" }),
        "build",
    )
    .await;
    let locked = snippet["result"]["lockedFilters"].as_array().unwrap();
    assert_eq!(locked.len(), 1, "{snippet}");
    assert_eq!(locked[0]["column"], "region", "{snippet}");
    // D7: a placeholder, never the saved value.
    assert_ne!(locked[0]["value"], json!("Bali"), "{snippet}");
    assert_no_secret(&snippet["result"]);
}

// ── withdraw_embed_tokens ──────────────────────────────────────────────

#[tokio::test]
async fn withdraw_needs_the_confirm_then_refuses_a_token_issued_before_it() {
    let (app, _ch) = app(Some(SECRET), Store::with_earlier_site()).await;
    let (_user, cookie) = manager(&app).await;
    let asked = call(
        &app,
        &cookie,
        "withdraw_embed_tokens",
        json!({ "board": "b1" }),
        "build",
    )
    .await;
    assert_eq!(asked["outcome"], "needs_confirmation", "{asked}");
    assert!(
        asked["result"]["summary"]
            .as_str()
            .unwrap_or_default()
            .contains("b1"),
        "{asked}"
    );

    let iat = (unix_now() - 5.0).floor();
    let token = sign_embed(
        &EmbedClaims {
            resource: Some(EmbedResource {
                chart: None,
                dashboard: Some("b2".to_owned()),
            }),
            params: None,
            exp: Some(iat + 3600.0),
            iat: Some(iat),
            jti: None,
        },
        SECRET,
    );
    let open = embed_data(&app, &token).await;
    assert_eq!(open.0, StatusCode::OK, "{}", open.1);

    let confirmed = call(
        &app,
        &cookie,
        "withdraw_embed_tokens",
        json!({ "board": "b2", "confirmed": true }),
        "build",
    )
    .await;
    assert_eq!(confirmed["outcome"], "executed", "{confirmed}");
    assert!(
        confirmed["result"]["embedRevokedBefore"].is_number(),
        "{confirmed}"
    );
    assert_no_secret(&confirmed["result"]);

    let refused = embed_data(&app, &token).await;
    assert_eq!(refused.0, StatusCode::UNAUTHORIZED, "{}", refused.1);
    assert_eq!(
        refused.1["error"], "embed token is invalid or expired",
        "{}",
        refused.1
    );
}

async fn embed_data(app: &TestApp, token: &str) -> (StatusCode, Value) {
    let resp = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/embed/data")
                .header("content-type", "application/json")
                .body(Body::from(json!({ "jwt": token }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

// ── get_embed_snippet ──────────────────────────────────────────────────

#[tokio::test]
async fn the_snippet_needs_manage_names_a_chart_and_the_address_template() {
    let (app, _ch) = app(Some(SECRET), Store::with_earlier_site()).await;
    let (_user, cookie) = manager(&app).await;
    let dashboard = call(
        &app,
        &cookie,
        "get_embed_snippet",
        json!({ "board": "b1" }),
        "build",
    )
    .await;
    assert_eq!(
        dashboard["result"]["resource"],
        json!({ "dashboard": "b1" }),
        "{dashboard}"
    );
    let chart = call(
        &app,
        &cookie,
        "get_embed_snippet",
        json!({ "board": "b1", "chart": "c1" }),
        "build",
    )
    .await;
    assert_eq!(
        chart["result"]["resource"],
        json!({ "chart": "c1" }),
        "{chart}"
    );
    assert!(
        dashboard["result"]["addressTemplate"]
            .as_str()
            .unwrap_or_default()
            .contains("TOKEN FROM YOUR SERVER"),
        "{dashboard}"
    );
    assert!(
        dashboard["result"]["note"]
            .as_str()
            .unwrap_or_default()
            .contains("Share > Embed"),
        "{dashboard}"
    );

    // A viewer with no manage is refused.
    let viewer = create_assistant_principal(&app.pool, "dashboard:read").await;
    let viewer_cookie = session_cookie_for_user(&app.pool, viewer).await;
    let refused = call(
        &app,
        &viewer_cookie,
        "get_embed_snippet",
        json!({ "board": "b1" }),
        "build",
    )
    .await;
    assert!(
        refused["result"]["error"]
            .as_str()
            .unwrap_or_default()
            .contains("manage on this dashboard"),
        "{refused}"
    );
    assert_no_secret(&dashboard["result"]);
    assert_no_secret(&chart["result"]);
}
